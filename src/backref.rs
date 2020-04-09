use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ffi::c_void;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::Deref;
use std::pin::Pin;
use std::rc::{Rc, Weak};

struct ByPointer<T>(*const T);

struct AnyBox {
    box_ptr: *mut c_void,
    drop_fn: unsafe fn(*mut c_void) -> (),
}

unsafe fn drop_box<T>(p: *mut c_void) {
    let p_typed: *mut T = p as *mut T;
    drop(Box::from_raw(p_typed));
}

impl AnyBox {
    fn new<T>(val: Box<T>) -> AnyBox {
        unsafe {
            let box_ptr: *mut T = Box::into_raw(val);
            AnyBox {
                box_ptr: std::mem::transmute(box_ptr),
                drop_fn: (drop_box::<T>),
            }
        }
    }

    unsafe fn borrow<T>(&self) -> &T {
        std::mem::transmute(self.box_ptr)
    }

    unsafe fn borrow_mut<T>(&mut self) -> &mut T {
        std::mem::transmute(self.box_ptr)
    }
}

impl Drop for AnyBox {
    fn drop(&mut self) {
        unsafe {
            (self.drop_fn)(self.box_ptr);
        }
    }
}

struct RInner {
    destructors: RefCell<HashMap<ByPointer<c_void>, Box<dyn Destructor>>>,
    inner: AnyBox,
}

pub struct R<T>(Rc<RInner>, PhantomData<T>);

impl<T> R<T> {
    pub fn new(t: Box<T>) -> Self {
        R::<T>(
            Rc::new(RInner {
                destructors: RefCell::new(HashMap::new()),
                inner: AnyBox::new(t),
            }),
            PhantomData,
        )
    }
}

impl<T> Clone for R<T> {
    fn clone(&self) -> Self {
        R(self.0.clone(), PhantomData)
    }
}

impl<T> Deref for R<T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { self.0.inner.borrow() }
    }
}

trait Destructor {
    fn destruct(&self, r: ByPointer<c_void>);
}

impl RInner {
    fn add_destructor(&self, k: *const c_void, d: Box<dyn Destructor>) {
        self.destructors
            .borrow_mut()
            .entry(ByPointer(k))
            .or_insert(d);
    }

    fn remove_destructor(&self, k: *const c_void) {
        self.destructors.borrow_mut().remove(&ByPointer(k));
    }
}

impl Drop for RInner {
    fn drop(&mut self) {
        let self_ref: &Self = &*self;
        let self_ptr: *const c_void = unsafe { std::mem::transmute(self_ref) };
        for (_, d) in self.destructors.borrow_mut().iter() {
            d.destruct(ByPointer(self_ptr));
        }
    }
}

impl<T> PartialEq for ByPointer<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<T> Eq for ByPointer<T> {}

impl<T> Hash for ByPointer<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state)
    }
}

struct ExpandoInner<T: 'static, V: 'static> {
    this: Weak<RInner>,
    on_remove: (&'static dyn Fn(R<T>, V) -> ()),
    items: RefCell<HashMap<ByPointer<c_void>, (V, Weak<RInner>)>>,
}

unsafe fn to_addr<T>(p: &Pin<Box<T>>) -> *mut T {
    let m: &T = &*p;
    m as *const T as *mut T
}

unsafe fn to_addr_rc<T>(p: &Rc<T>) -> *mut T {
    let m: &T = &*p;
    m as *const T as *mut T
}

unsafe fn to_cvoid<T>(t: *const T) -> *const c_void {
    t as *const c_void
}

struct ExpandoBase<T: 'static, V: 'static>(Pin<Box<ExpandoInner<T, V>>>);

impl<T, V> Destructor for *mut ExpandoInner<T, V> {
    fn destruct(&self, r: ByPointer<c_void>) {
        let self_ref = unsafe { &**self };
        let mut items = self_ref.items.borrow_mut();
        match items.entry(r) {
            Entry::Vacant(_) => panic!("destructing value that was not set (?)"),
            Entry::Occupied(entry) => {
                let (_, (v, _)) = entry.remove_entry();
                match self_ref.this.upgrade() {
                    Some(this_strong) => (self_ref.on_remove)(R(this_strong, PhantomData), v),
                    None => (),
                }
            }
        }
    }
}

impl<T, V> ExpandoBase<T, V> {
    fn add_destructor_callback(&self, a: &RInner) {
        unsafe { a.add_destructor(to_cvoid(to_addr(&self.0)), Box::new(to_addr(&self.0))) }
    }

    fn remove_destructor_callback(&self, a: Rc<RInner>) {
        unsafe { a.remove_destructor(to_cvoid(to_addr(&self.0))) }
    }

    pub fn iter(&self, cb: &mut dyn FnMut(Rc<RInner>, &V) -> ()) {
        let items = self.0.items.borrow();
        for (_, (v, weak_ref)) in items.iter() {
            cb(weak_ref.upgrade().unwrap(), v)
        }
    }

    pub fn update(&self, a: Rc<RInner>, value_f: impl FnOnce(Option<V>) -> V) {
        let mut items = self.0.items.borrow_mut();
        let key = ByPointer(unsafe { to_cvoid(to_addr_rc(&a)) });
        let key1 = ByPointer(unsafe { to_cvoid(to_addr_rc(&a)) }); // sigh
        match items.entry(key) {
            Entry::Vacant(entry) => {
                self.add_destructor_callback(&*a);
                entry.insert((value_f(None), Rc::downgrade(&a)));
            }
            Entry::Occupied(entry) => {
                let new_value = value_f(Some(entry.remove().0));
                items.insert(key1, (new_value, Rc::downgrade(&a)));
            }
        }
    }

    pub fn add(&self, a: Rc<RInner>, value: V) {
        self.update(a, |_old_value| value);
    }

    pub fn new(this: R<T>, on_remove: (&'static dyn Fn(R<T>, V) -> ())) -> Self {
        let inner = ExpandoInner {
            this: Rc::downgrade(&this.0),
            on_remove,
            items: RefCell::new(HashMap::new()),
        };
        ExpandoBase(Box::pin(inner))
    }

    pub fn get_this(&self) -> R<T> {
        R(self.0.this.upgrade().unwrap(), PhantomData)
    }
}

impl<T, V> ExpandoBase<T, Vec<V>> {
    pub fn add_multi(&self, a: Rc<RInner>, value: V) {
        self.update(a, |old_value| match old_value {
            None => vec![value],
            Some(mut l) => {
                l.push(value);
                l
            }
        })
    }
}

impl<T, V> Drop for ExpandoBase<T, V> {
    fn drop(&mut self) {
        let items = (*self).0.items.borrow();
        for (_, (_, weak)) in items.iter() {
            self.remove_destructor_callback(Weak::upgrade(weak).unwrap())
        }
    }
}

struct CallbackEntry<ArgType> {
    invoker: unsafe fn(*const c_void, Rc<RInner>, &ArgType) -> (),
    fun_ptr: *const c_void,
}

pub struct CallbackExpando<ArgType: 'static>(ExpandoBase<(), Vec<CallbackEntry<ArgType>>>);

unsafe fn invoker<K, ArgType>(fun_ptr: *const c_void, key: Rc<RInner>, arg: &ArgType) {
    let fun_ptr: (fn(R<K>, &ArgType) -> ()) = std::mem::transmute(fun_ptr);
    fun_ptr(R::<K>(key, PhantomData), arg)
}

impl<ArgType> CallbackExpando<ArgType> {
    pub fn add_callback<K>(&self, key: R<K>, f: fn(R<K>, &ArgType) -> ()) {
        let value = CallbackEntry {
            invoker: (invoker::<K, ArgType>),
            fun_ptr: unsafe { std::mem::transmute(f) },
        };

        self.0.add_multi(key.0, value);
    }

    pub fn call(&self, arg: &ArgType) {
        self.0.iter(&mut |k, v| {
            for entry in v {
                unsafe {
                    (entry.invoker)(entry.fun_ptr, k.clone(), arg);
                }
            }
        });
    }

    pub fn new() -> Self {
        CallbackExpando(ExpandoBase::new(R::new(Box::new(())), &|_, _| {}))
    }
}
