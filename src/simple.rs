use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use super::backref::*;
use super::common::*;

pub struct System {
    callback_queue: RefCell<VecDeque<Box<dyn FnOnce() -> ()>>>,
}

impl System {
    pub fn new() -> Rc<System> {
        Rc::new(System {
            callback_queue: RefCell::new(VecDeque::new()),
        })
    }

    pub fn run(&self) {
        loop {
            let item = self.callback_queue.borrow_mut().pop_front();
            match item {
                None => break,
                Some(callback) => callback(),
            }
        }
    }
}

pub struct Listeners<T: Row> {
    sys: Rc<System>,
    mapping: CallbackExpando<(T, i64)>,
}

impl<T: Row> Listeners<T> {
    fn new(sys: Rc<System>) -> Rc<Listeners<T>> {
        Rc::new(Listeners {
            sys,
            mapping: CallbackExpando::new(),
        })
    }

    fn delta(self: Rc<Listeners<T>>, t: T, delta: i64) {
        let self1 = self.clone();
        let impl_delta_cb = move || {
            self1.mapping.call(&(t, delta));
        };

        self.sys
            .callback_queue
            .borrow_mut()
            .push_back(Box::new(impl_delta_cb));
    }
}

pub trait RelImpl<T: Row> {
    fn iter_values(&self, f: &mut dyn FnMut(&T) -> ());

    fn get_listeners(&self) -> &Listeners<T>;
}

#[derive(Clone)]
pub struct Rel<T: Row>(Rc<dyn RelImpl<T>>);

impl<T: Row> Rel<T> {
    pub fn iter_values(&self, f: &mut dyn FnMut(&T) -> ()) {
        self.0.iter_values(f);
    }

    pub fn to_vec(&self) -> Vec<T> {
        let mut result = Vec::new();
        self.iter_values(&mut |val| result.push(val.clone()));
        result
    }

    pub fn add_listener<A>(&self, this: R<A>, f: fn(R<A>, &(T, i64)) -> ()) {
        self.0.get_listeners().mapping.add_callback(this, f);
    }
}

impl<'a, T: Row + Ord> Rel<T> {
    pub fn to_sorted_vec(self) -> Vec<T> {
        let mut result = self.to_vec();
        result.sort();
        result
    }
}

pub mod memo_rel {
    use super::super::backref::*;
    use super::super::common::*;
    use super::*;

    use std::collections::HashMap;

    struct MemoRel<T: Row> {
        listeners: Rc<Listeners<T>>,
        vals: RefCell<HashMap<T, i64>>,
    }

    struct MemoRelImpl<T: Row>(R<MemoRel<T>>);

    impl<T: Row> RelImpl<T> for MemoRelImpl<T> {
        fn get_listeners(&self) -> &Listeners<T> {
            &self.0.listeners
        }

        fn iter_values(&self, f: &mut dyn FnMut(&T) -> ()) {
            for (k, count) in self.0.vals.borrow().iter() {
                if *count != 0 {
                    f(k);
                }
            }
        }
    }

    fn on_delta<T: Row>(this: R<MemoRel<T>>, x: &(T, i64)) {
        let (t, delta) = x;
        let delta = *delta;
        println!("memorel/delta");
        // TOOD: remove if zero
        assert!(delta != 0);
        let mut vals_mut = this.vals.borrow_mut();
        let current_count = vals_mut.entry(t.clone()).or_insert(0);
        let is_added = *current_count == 0;
        *current_count += delta;
        let is_removed = *current_count == 0;
        if is_added {
            this.listeners.clone().delta(t.clone(), 1);
        } else if is_removed {
            this.listeners.clone().delta(t.clone(), -1);
        }
    }

    pub fn new<T: Row>(sys: Rc<System>, rel: &Rel<T>) -> Rel<T> {
        let self_rel: R<MemoRel<T>> = R::new(Box::new(MemoRel {
            listeners: Listeners::new(sys),
            vals: RefCell::new(HashMap::new()),
        }));
        let self_rel1 = self_rel.clone();
        rel.iter_values(&mut |value| {
            on_delta(self_rel1.clone(), &(value.clone(), 1));
        });
        rel.clone().add_listener(self_rel.clone(), on_delta);
        Rel(Rc::new(MemoRelImpl(self_rel)))
    }
}

pub mod data_rel {
    use super::super::backref::*;
    use super::super::common::*;
    use super::*;

    use std::collections::hash_map::Entry;
    use std::collections::HashMap;

    pub struct DataRel<T: Row> {
        listeners: Rc<Listeners<T>>,
        vals: RefCell<HashMap<T, ()>>,
    }

    struct DataRelImpl<T: Row>(R<DataRel<T>>);

    impl<T: Row> DataRel<T> {
        pub fn add(&self, t: &T) {
            let mut vals_mut = self.vals.borrow_mut();
            let entry = vals_mut.entry(t.clone());

            match entry {
                Entry::Occupied(_) => (),
                Entry::Vacant(entry) => {
                    entry.insert(());
                    self.listeners.clone().delta(t.clone(), 1)
                }
            }
        }

        pub fn remove(&self, t: &T) {
            let mut vals_mut = self.vals.borrow_mut();
            let entry = vals_mut.entry(t.clone());
            match entry {
                Entry::Vacant(_) => (),
                Entry::Occupied(entry) => {
                    entry.remove();
                    self.listeners.clone().delta(t.clone(), -1);
                }
            }
        }
    }

    impl<T: Row> RelImpl<T> for DataRelImpl<T> {
        fn get_listeners(&self) -> &Listeners<T> {
            &self.0.listeners
        }

        fn iter_values(&self, f: &mut dyn FnMut(&T) -> ()) {
            for (k, ()) in self.0.vals.borrow().iter() {
                f(k);
            }
        }
    }

    pub fn new<T: Row>(sys: Rc<System>, initial: &[T]) -> (R<DataRel<T>>, Rel<T>) {
        let self_rel: R<DataRel<T>> = R::new(Box::new(DataRel {
            listeners: Listeners::new(sys),
            vals: RefCell::new(HashMap::new()),
        }));
        let rel = Rel(Rc::new(DataRelImpl(self_rel.clone())));
        for item in initial {
            self_rel.add(item);
        }
        (self_rel, rel)
    }
}

pub mod map_rel {

    use super::super::backref::*;
    use super::super::common::*;
    use super::*;

    pub struct MapRel<T: Row, TR: Row> {
        listeners: Rc<Listeners<TR>>,
        rel: Rel<T>,
        f: Box<dyn Fn(&T) -> TR>,
    }

    struct MapRelImpl<T: Row, TR: Row>(R<MapRel<T, TR>>);

    impl<T: Row, TR: Row> RelImpl<TR> for MapRelImpl<T, TR> {
        fn get_listeners(&self) -> &Listeners<TR> {
            &self.0.listeners
        }

        fn iter_values(&self, f: &mut dyn FnMut(&TR) -> ()) {
            self.0.rel.iter_values(&mut |val| {
                f(&(self.0.f)(val));
            })
        }
    }

    fn on_delta<T: Row, TR: Row>(this: R<MapRel<T, TR>>, x: &(T, i64)) {
        let (t, delta) = x;
        let delta = *delta;
        let mapped_value = (this.f)(t);
        this.listeners.clone().delta(mapped_value, delta);
    }

    pub fn new<T: Row, TR: Row>(
        sys: Rc<System>,
        rel: &Rel<T>,
        f: Box<dyn Fn(&T) -> TR>,
    ) -> Rel<TR> {
        let self_rel = R::new(Box::new(MapRel {
            listeners: Listeners::new(sys),
            rel: rel.clone(),
            f,
        }));
        rel.clone().add_listener(self_rel.clone(), on_delta);
        Rel(Rc::new(MapRelImpl(self_rel)))
    }
}
