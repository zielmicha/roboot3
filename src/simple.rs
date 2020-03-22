use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::rc::{Rc, Weak};

use super::backref::*;
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
    mapping: Expando<(), Box<dyn Listener<T>>, ()>,
}

impl<T: Row> Listeners<T> {
    fn new(sys: Rc<System>) -> Rc<Listeners<T>> {
        Rc::new(Listeners {
            sys,
            mapping: Expando::new(R::new(Box::new(())), &|_, _| ()),
        })
    }

    fn delta(self: Rc<Listeners<T>>, t: T, delta: i64) {
        let self1 = self.clone();
        let impl_delta_cb = move || {
            self1
                .mapping
                .iter(&mut |listener: R<Box<dyn Listener<T>>>, &()| {
                    listener.on_delta(&t, delta);
                })
        };

        self.sys
            .callback_queue
            .borrow_mut()
            .push_back(Box::new(impl_delta_cb));
    }
}

pub trait Rel<T: Row> {
    fn iter_values(&self, f: &mut dyn FnMut(&T) -> ());

    fn get_listeners(&self) -> &Listeners<T>;
}

impl<T: Row> dyn Rel<T> {
    pub fn to_vec(&self) -> Vec<T> {
        let mut result = Vec::new();
        self.iter_values(&mut |val| result.push(val.clone()));
        result
    }
}

struct MemoRel<T: Row> {
    listeners: Rc<Listeners<T>>,
    vals: RefCell<HashMap<T, i64>>,
}

impl<T: Row> Rel<T> for MemoRel<T> {
    fn get_listeners(&self) -> &Listeners<T> {
        &self.listeners
    }

    fn iter_values(&self, f: &mut dyn FnMut(&T) -> ()) {
        for (k, count) in self.vals.borrow().iter() {
            if *count != 0 {
                f(k);
            }
        }
    }
}

pub fn memo<T: Row>(sys: Rc<System>, rel: R<dyn Rel<T>>) -> R<dyn Rel<T>> {
    // let a: Rc<MemoRel<T>> = Rc::new(MemoRel {
    //     listeners: Listeners::new(sys),
    //     vals: RefCell::new(HashMap::new()),
    // });
    //let b: Rc<dyn Rel<T>> = a;
    //
    let self_rel: R<MemoRel<T>> = R::new(Box::new(MemoRel {
        listeners: Listeners::new(sys),
        vals: RefCell::new(HashMap::new()),
    }));
    self_rel
}
