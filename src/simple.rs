use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::collections::VecDeque;

use super::common::*;

trait Listener<'a, T> {
    fn on_delta(&'a self, t: &T, delta: i64);
}

pub struct System<'a> {
    callback_queue: RefCell<VecDeque<Box<dyn 'a + FnOnce() -> ()>>>,
    arena: super::arena::Arena<'a>,
}

impl<'a> System<'a> {
    pub fn new() -> System<'a> {
        System {
            callback_queue: RefCell::new(VecDeque::new()),
            arena: super::arena::Arena::new(),
        }
    }

    pub fn alloc<T>(&'a self, a: T) -> &'a T {
        self.arena.alloc(a)
    }
}

pub fn with_system<'a, T>(f: impl FnOnce(&'a System<'a>) -> T) -> T {
    let sys: System<'a> = System::new();
    let sys_ref: &System<'a> = &sys;
    // the lifetime of sys_ref is *slightly* shorter, but we know that sys_ref_ok will not be used
    // anyway after [f] returns, so this is safe.
    let sys_ref_ok: &'a System<'a> = unsafe { std::mem::transmute(sys_ref) };
    f(sys_ref_ok)
}

pub trait RelMeta<'a, T: Row> {
    fn iter_values(&'a self, f: &dyn Fn(&T) -> ());
}

pub struct RelImpl<'a, T> {
    sys: &'a System<'a>,
    listeners: RefCell<Vec<&'a dyn Listener<'a, T>>>,
}

pub struct Rel<'a, T> {
    rel_impl: &'a RelImpl<'a, T>,
    pub meta: &'a dyn RelMeta<'a, T>,
}

impl<'a, T> Clone for Rel<'a, T> {
    fn clone(&self) -> Self {
        Rel {
            rel_impl: self.rel_impl,
            meta: self.meta,
        }
    }
}

impl<'a, T> Copy for Rel<'a, T> {}

impl<'a, T: Row> RelImpl<'a, T> {
    pub fn new(sys: &'a System<'a>) -> &'a RelImpl<'a, T> {
        sys.arena.alloc(RelImpl {
            sys,
            listeners: RefCell::new(Vec::new()),
        })
    }

    fn make_impl_delta_cb(&'a self, t: T, delta: i64) -> Box<dyn 'a + FnOnce() -> ()> {
        Box::new(move || {
            let b = self.listeners.borrow();
            for listener_box in b.iter() {
                listener_box.on_delta(&t, delta);
            }
        })
    }

    fn impl_delta(&'a self, t: &T, delta: i64) {
        let impl_delta_cb = self.make_impl_delta_cb(t.clone(), delta);

        self.sys
            .callback_queue
            .borrow_mut()
            .push_back(impl_delta_cb);
    }

    fn with_meta(&'a self, meta: &'a dyn RelMeta<'a, T>) -> Rel<'a, T> {
        Rel {
            rel_impl: self,
            meta,
        }
    }
}

impl<'a, T: Row> Rel<'a, T> {
    fn add_listener(&self, f: &'a dyn Listener<'a, T>) {
        self.rel_impl.listeners.borrow_mut().push(f)
    }
}

pub struct DataRel<'a, T: Row> {
    rel: &'a RelImpl<'a, T>,
    vals: RefCell<HashMap<T, ()>>,
}

impl<'a, T: Row> RelMeta<'a, T> for DataRel<'a, T> {
    fn iter_values(&'a self, f: &dyn Fn(&T) -> ()) {
        for (k, ()) in self.vals.borrow().iter() {
            f(k);
        }
    }
}

impl<'a, T: Row> DataRel<'a, T> {
    pub fn add(&'a self, t: &T) {
        let mut vals_mut = self.vals.borrow_mut();
        let entry = vals_mut.entry(t.clone());

        match entry {
            Entry::Occupied(_) => (),
            Entry::Vacant(entry) => {
                entry.insert(());
                self.rel.impl_delta(t, 1)
            }
        }
    }

    pub fn remove(&'a self, t: &T) {
        let mut vals_mut = self.vals.borrow_mut();
        let entry = vals_mut.entry(t.clone());
        match entry {
            Entry::Vacant(_) => (),
            Entry::Occupied(entry) => {
                entry.remove();
                self.rel.impl_delta(t, -1);
            }
        }
    }

    pub fn rel(&'a self) -> Rel<'a, T> {
        self.rel.with_meta(self)
    }
}

pub fn data_rel<'a, T: Row>(sys: &'a System<'a>, initial: &[T]) -> &'a DataRel<'a, T> {
    let result = RelImpl::new(sys);
    let this = sys.arena.alloc(DataRel {
        rel: result,
        vals: RefCell::new(HashMap::new()),
    });
    for item in initial {
        this.add(item);
    }
    this
}

struct MemoRel<'a, T: Row> {
    rel: &'a RelImpl<'a, T>,
    vals: RefCell<HashMap<T, i64>>,
}

impl<'a, T: Row> Listener<'a, T> for MemoRel<'a, T> {
    fn on_delta(&'a self, t: &T, delta: i64) {
        // TOOD: remove if zero
        assert!(delta != 0);
        let mut vals_mut = self.vals.borrow_mut();
        let current_count = vals_mut.entry(t.clone()).or_insert(0);
        let is_added = *current_count == 0;
        *current_count += delta;
        let is_removed = *current_count == 0;
        if is_added {
            self.rel.impl_delta(t, 1);
        } else if is_removed {
            self.rel.impl_delta(t, -1);
        }
    }
}

impl<'a, T: Row> RelMeta<'a, T> for MemoRel<'a, T> {
    fn iter_values(&'a self, f: &dyn Fn(&T) -> ()) {
        for (k, count) in self.vals.borrow().iter() {
            if *count != 0 {
                f(k);
            }
        }
    }
}

pub fn memo<'a, T: Row>(sys: &'a System<'a>, rel: &'a Rel<'a, T>) -> Rel<'a, T> {
    let result = RelImpl::new(sys);
    let this = sys.arena.alloc(MemoRel {
        rel: result,
        vals: RefCell::new(HashMap::new()),
    });
    rel.add_listener(this);
    result.with_meta(this)
}

struct MapRel<'a, T, R> {
    rel: Rel<'a, T>,
    result: &'a RelImpl<'a, R>,
    f: &'a dyn Fn(T) -> R,
}

impl<'a, T: Row, R: Row> Listener<'a, T> for MapRel<'a, T, R> {
    fn on_delta(&'a self, t: &T, delta: i64) {
        self.result.impl_delta(&(self.f)(t.clone()), delta)
    }
}

impl<'a, T: Row, R: Row> RelMeta<'a, R> for MapRel<'a, T, R> {
    fn iter_values(&'a self, f: &dyn Fn(&R) -> ()) {
        self.rel.meta.iter_values(&|val| {
            f(&(self.f)(val.clone()));
        })
    }
}

pub fn map<'a, T: Row, R: Row>(
    sys: &'a System<'a>,
    rel: Rel<'a, T>,
    f: &'a dyn Fn(T) -> R,
) -> Rel<'a, R> {
    let result = RelImpl::new(sys);
    let this = sys.arena.alloc(MapRel { rel, result, f });
    rel.add_listener(this);
    result.with_meta(this)
}

struct JoinRel<'a, A: Row, B: Row, K: Row> {
    rel: &'a RelImpl<'a, (A, B)>,
    key_a: Box<dyn Fn(&A) -> K>,
    key_b: Box<dyn Fn(&B) -> K>,
    val_a: RefCell<HashMap<K, HashMap<A, i64>>>,
    val_b: RefCell<HashMap<K, HashMap<B, i64>>>,
}

impl<'a, A: Row, B: Row, K: Row> JoinRel<'a, A, B, K> {
    fn on_real_delta_a(&'a self, key: &K, t: &A, delta: i64) {
        if let Some(other_list) = self.val_b.borrow().get(key) {
            for (other, _) in other_list {
                self.rel.impl_delta(&(t.clone(), other.clone()), delta);
            }
        }
    }

    fn on_real_delta_b(&'a self, key: &K, t: &B, delta: i64) {
        if let Some(other_list) = self.val_a.borrow().get(key) {
            for (other, _) in other_list {
                self.rel.impl_delta(&(other.clone(), t.clone()), delta);
            }
        }
    }
}

struct JoinRelA<'a, A: Row, B: Row, K: Row>(&'a JoinRel<'a, A, B, K>);
struct JoinRelB<'a, A: Row, B: Row, K: Row>(&'a JoinRel<'a, A, B, K>);

impl<'a, A: Row, B: Row, K: Row> Listener<'a, A> for JoinRelA<'a, A, B, K> {
    fn on_delta(&'a self, t: &A, delta: i64) {
        assert!(delta != 0);
        let rel = self.0;
        let key = (rel.key_a)(t);
        // todo: remove empty entries, clone()s are not needed in some cases
        let mut val_a = rel.val_a.borrow_mut();
        let current_count = val_a
            .entry(key.clone())
            .or_insert(HashMap::new())
            .entry(t.clone())
            .or_insert(0);
        let is_added = *current_count == 0;
        *current_count += delta;
        let is_deleted = *current_count == 0;
        if is_added {
            rel.on_real_delta_a(&key, t, 1);
        } else if is_deleted {
            rel.on_real_delta_a(&key, t, -1);
        }
    }
}

impl<'a, A: Row, B: Row, K: Row> Listener<'a, B> for JoinRelB<'a, A, B, K> {
    fn on_delta(&'a self, t: &B, delta: i64) {
        assert!(delta != 0);
        let rel = self.0;
        let key = (rel.key_b)(t);
        // todo: remove empty entries, clone()s are not needed in some cases
        let mut val_b = rel.val_b.borrow_mut();
        let current_count = val_b
            .entry(key.clone())
            .or_insert(HashMap::new())
            .entry(t.clone())
            .or_insert(0);
        let is_added = *current_count == 0;
        *current_count += delta;
        let is_deleted = *current_count == 0;
        if is_added {
            rel.on_real_delta_b(&key, t, -1);
        } else if is_deleted {
            rel.on_real_delta_b(&key, t, 1);
        }
    }
}

impl<'a, A: Row, B: Row, K: Row> RelMeta<'a, (A, B)> for JoinRel<'a, A, B, K> {
    fn iter_values(&'a self, f: &dyn Fn(&(A, B)) -> ()) {
        for (key, values_a) in self.val_a.borrow().iter() {
            match self.val_b.borrow().get(&key) {
                None => (),
                Some(values_b) => {
                    for (val_a, _) in values_a {
                        for (val_b, _) in values_b {
                            f(&(val_a.clone(), val_b.clone()));
                        }
                    }
                }
            }
        }
    }
}

pub fn join<'a, A: Row, B: Row, K: Row>(
    sys: &'a System<'a>,
    rel_a: Rel<'a, A>,
    rel_b: Rel<'a, B>,
    key_a: Box<dyn Fn(&A) -> K>,
    key_b: Box<dyn Fn(&B) -> K>,
) -> Rel<'a, (A, B)> {
    let rel = RelImpl::new(sys);
    let this = sys.arena.alloc(JoinRel {
        rel,
        key_a: key_a,
        key_b: key_b,
        val_a: RefCell::new(HashMap::new()),
        val_b: RefCell::new(HashMap::new()),
    });
    rel_a.add_listener(sys.arena.alloc(JoinRelA(this)));
    rel_b.add_listener(sys.arena.alloc(JoinRelB(this)));
    rel.with_meta(this)
}
