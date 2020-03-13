use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::rc::Rc;

use super::common::*;

trait Listener<T> {
    fn on_delta(&self, t: &T, delta: i64);
}

pub struct System {
    callback_queue: RefCell<VecDeque<Box<dyn FnOnce() -> ()>>>,
}

impl System {
    pub fn new() -> System {
        System {
            callback_queue: RefCell::new(VecDeque::new()),
        }
    }
}

pub struct Rel<T> {
    sys: System,
    listeners: RefCell<Vec<Box<dyn Listener<T>>>>,
}

impl<T> Rel<T> {
    fn impl_delta(&self, t: &T, delta: i64) {
        let f = || {
            for listener in self.listeners.borrow().iter() {
                listener.on_delta(t, delta);
            }
        };
        f();
        //self.sys.callback_queue.borrow_mut().push_back(Box::new(f))
    }

    fn add_listener(&self, f: Box<dyn Listener<T>>) {
        self.listeners.borrow_mut().push(f)
    }
}

impl<'a, T> Rel<T> {
    fn impl_new(sys: System) -> Rc<Rel<T>> {
        Rc::new(Rel {
            sys,
            listeners: RefCell::new(Vec::new()),
        })
    }
}

pub struct DataRel<T: Row> {
    pub rel: Rc<Rel<T>>,
    vals: HashMap<T, ()>,
}

impl<T: Row> DataRel<T> {
    pub fn add(&mut self, t: &T) {
        let entry = self.vals.entry(t.clone());

        match entry {
            Entry::Occupied(_) => (),
            Entry::Vacant(entry) => {
                entry.insert(());
                self.rel.impl_delta(t, 1)
            }
        }
    }

    pub fn remove(&mut self, t: &T) {
        let entry = self.vals.entry(t.clone());

        match entry {
            Entry::Vacant(_) => (),
            Entry::Occupied(entry) => {
                entry.remove();
                self.rel.impl_delta(t, -1);
            }
        }
    }
}

pub fn data_rel<T: Row>(sys: System, initial: &[T]) -> DataRel<T> {
    let result = Rel::impl_new(sys);
    let mut this = DataRel {
        rel: result.clone(),
        vals: HashMap::new(),
    };
    for item in initial {
        this.add(item);
    }
    this
}

struct MemoRel<T: Row> {
    rel: Rc<Rel<T>>,
    vals: HashMap<T, i64>,
}

impl<T: Row> Listener<T> for RefCell<MemoRel<T>> {
    fn on_delta(&self, t: &T, delta: i64) {
        // TOOD: remove if zero
        assert!(delta != 0);
        let mut self_mut = self.borrow_mut();
        let current_count = self_mut.vals.entry(t.clone()).or_insert(0);
        let is_added = *current_count == 0;
        *current_count += delta;
        let is_removed = *current_count == 0;
        if is_added {
            self_mut.rel.impl_delta(t, 1);
        } else if is_removed {
            self_mut.rel.impl_delta(t, -1);
        }
    }
}

pub fn memo<T: Row>(sys: System, rel: Rc<Rel<T>>) -> Rc<Rel<T>> {
    let result = Rel::impl_new(sys);
    let this = MemoRel {
        rel: result.clone(),
        vals: HashMap::new(),
    };
    rel.add_listener(Box::new(RefCell::new(this)));
    result
}

struct MapRel<T, R> {
    rel: Rc<Rel<R>>,
    f: Box<dyn Fn(&T) -> R>,
}

impl<T, R> Listener<T> for Rc<MapRel<T, R>> {
    fn on_delta(&self, t: &T, delta: i64) {
        self.rel.impl_delta(&(self.f)(t), delta)
    }
}

pub fn map<T: Row, R: Row>(sys: System, rel: Rc<Rel<T>>, f: Box<dyn Fn(&T) -> R>) -> Rc<Rel<R>> {
    let result = Rel::impl_new(sys);
    let this = Rc::new(MapRel {
        rel: result.clone(),
        f: f,
    });
    rel.add_listener(Box::new(this));
    result
}

struct JoinRel<A: Row, B: Row, K: Row> {
    rel: Rc<Rel<(A, B)>>,
    key_a: Box<dyn Fn(&A) -> K>,
    key_b: Box<dyn Fn(&B) -> K>,
    val_a: HashMap<K, HashMap<A, i64>>,
    val_b: HashMap<K, HashMap<B, i64>>,
}

impl<A: Row, B: Row, K: Row> JoinRel<A, B, K> {
    fn on_real_delta_a(&mut self, key: &K, t: &A, delta: i64) {
        if let Some(other_list) = self.val_b.get(key) {
            for (other, _) in other_list {
                self.rel.impl_delta(&(t.clone(), other.clone()), delta);
            }
        }
    }

    fn on_real_delta_b(&mut self, key: &K, t: &B, delta: i64) {
        if let Some(other_list) = self.val_a.get(key) {
            for (other, _) in other_list {
                self.rel.impl_delta(&(other.clone(), t.clone()), delta);
            }
        }
    }
}

struct JoinRelA<A: Row, B: Row, K: Row>(Rc<RefCell<JoinRel<A, B, K>>>);
struct JoinRelB<A: Row, B: Row, K: Row>(Rc<RefCell<JoinRel<A, B, K>>>);

impl<A: Row, B: Row, K: Row> Listener<A> for JoinRelA<A, B, K> {
    fn on_delta(&self, t: &A, delta: i64) {
        assert!(delta != 0);
        let mut rel = (*self.0).borrow_mut();
        let key = (rel.key_a)(t);
        // todo: remove empty entries, clone()s are not needed in some cases
        let current_count = rel
            .val_a
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

impl<A: Row, B: Row, K: Row> Listener<B> for JoinRelB<A, B, K> {
    fn on_delta(&self, t: &B, delta: i64) {
        assert!(delta != 0);
        let mut rel = (*self.0).borrow_mut();
        let key = (rel.key_b)(t);
        // todo: remove empty entries, clone()s are not needed in some cases
        let current_count = rel
            .val_b
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

pub fn join<A: Row, B: Row, K: Row>(
    sys: System,
    rel_a: Rc<Rel<A>>,
    rel_b: Rc<Rel<B>>,
    key_a: Box<dyn Fn(&A) -> K>,
    key_b: Box<dyn Fn(&B) -> K>,
) -> Rc<Rel<(A, B)>> {
    let rel = Rel::impl_new(sys);
    let this = Rc::new(RefCell::new(JoinRel {
        rel: rel.clone(),
        key_a: key_a,
        key_b: key_b,
        val_a: HashMap::new(),
        val_b: HashMap::new(),
    }));
    rel_a.add_listener(Box::new(JoinRelA(this.clone())));
    rel_b.add_listener(Box::new(JoinRelB(this.clone())));
    rel
}
