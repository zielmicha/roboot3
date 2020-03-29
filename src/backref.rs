use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::ffi::c_void;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::pin::Pin;
use std::rc::{Rc, Weak};

struct ByPointer<T>(*const T);

struct RInner<T: ?Sized> {
    destructors: RefCell<HashMap<ByPointer<c_void>, Box<dyn Destructor>>>,
    inner: Box<T>,
}

pub struct R<T: ?Sized>(Rc<RInner<T>>);

impl<T: ?Sized> R<T> {
    pub fn new(t: Box<T>) -> Self {
        R(Rc::new(RInner {
            destructors: RefCell::new(HashMap::new()),
            inner: t,
        }))
    }
}

impl<T: ?Sized> Clone for R<T> {
    fn clone(&self) -> Self {
        R(self.0.clone())
    }
}

impl<T: ?Sized> Deref for R<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0.inner
    }
}

trait Destructor {
    fn destruct(&self, r: ByPointer<c_void>);
}

impl<T: ?Sized> RInner<T> {
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

impl<T: ?Sized> Drop for RInner<T> {
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

struct ExpandoInner<T: 'static, K: 'static + ?Sized, V: 'static> {
    this: Weak<RInner<T>>,
    on_remove: (&'static dyn Fn(R<T>, V) -> ()),
    items: RefCell<HashMap<ByPointer<c_void>, (V, Weak<RInner<K>>)>>,
}

unsafe fn to_addr<T: ?Sized>(p: &Pin<Box<T>>) -> *mut T {
    let m: &T = &*p;
    m as *const T as *mut T
}

unsafe fn to_addr_rc<T: ?Sized>(p: &Rc<T>) -> *mut T {
    let m: &T = &*p;
    m as *const T as *mut T
}

unsafe fn to_cvoid<T>(t: *const T) -> *const c_void {
    t as *const c_void
}

pub struct Expando<T: 'static, K: 'static + ?Sized, V: 'static>(Pin<Box<ExpandoInner<T, K, V>>>);

impl<T, K: ?Sized, V> Destructor for *mut ExpandoInner<T, K, V> {
    fn destruct(&self, r: ByPointer<c_void>) {
        let self_ref = unsafe { &**self };
        let mut items = self_ref.items.borrow_mut();
        match items.entry(r) {
            Entry::Vacant(_) => panic!("destructing value that was not set (?)"),
            Entry::Occupied(entry) => {
                let (_, (v, _)) = entry.remove_entry();
                match self_ref.this.upgrade() {
                    Some(this_strong) => (self_ref.on_remove)(R(this_strong), v),
                    None => (),
                }
            }
        }
    }
}

impl<T, K: ?Sized, V> Expando<T, K, V> {
    fn add_destructor_callback(&self, a: &RInner<K>) {
        unsafe { a.add_destructor(to_cvoid(to_addr(&self.0)), Box::new(to_addr(&self.0))) }
    }

    fn remove_destructor_callback(&self, a: Rc<RInner<K>>) {
        unsafe { a.remove_destructor(to_cvoid(to_addr(&self.0))) }
    }

    pub fn iter(&self, cb: &mut dyn FnMut(R<K>, &V) -> ()) {
        let items = self.0.items.borrow();
        for (_, (v, weak_ref)) in items.iter() {
            cb(R(weak_ref.upgrade().unwrap()), v)
        }
    }

    pub fn add(&self, a: R<K>, value: V) {
        let mut items = self.0.items.borrow_mut();
        let key = ByPointer(unsafe { to_cvoid(to_addr_rc(&a.0)) });
        match items.entry(key) {
            Entry::Vacant(entry) => {
                self.add_destructor_callback(&a.0);
                entry.insert((value, Rc::downgrade(&a.0)));
            }
            Entry::Occupied(mut entry) => {
                entry.insert((value, Rc::downgrade(&a.0)));
            }
        }
    }

    pub fn new(this: R<T>, on_remove: (&'static dyn Fn(R<T>, V) -> ())) -> Self {
        let inner = ExpandoInner {
            this: Rc::downgrade(&this.0),
            on_remove,
            items: RefCell::new(HashMap::new()),
        };
        Expando(Box::pin(inner))
    }
}

impl<T, K: ?Sized, V> Drop for Expando<T, K, V> {
    fn drop(&mut self) {
        let items = (*self).0.items.borrow();
        for (_, (_, weak)) in items.iter() {
            self.remove_destructor_callback(Weak::upgrade(weak).unwrap())
        }
    }
}
