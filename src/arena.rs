// based on https://github.com/DuckLogic/rust-dynamic-arena/blob/master/src/lib.rs
use std::cell::RefCell;
use std::marker;
use std::os::raw::c_void;
use std::{mem, ptr};

pub struct Arena<'a> {
    handle: typed_arena::Arena<u8>,
    items: RefCell<Vec<DynamicArenaItem>>,
    _marker: marker::PhantomData<*mut &'a ()>,
}

struct DynamicArenaItem {
    drop: unsafe fn(*mut c_void),
    value: *mut c_void,
}

impl Drop for DynamicArenaItem {
    #[inline]
    fn drop(&mut self) {
        unsafe { (self.drop)(self.value) }
    }
}

impl<'a> Arena<'a> {
    pub fn new() -> Arena<'a> {
        Arena {
            handle: typed_arena::Arena::new(),
            items: RefCell::new(Vec::new()),
            _marker: marker::PhantomData,
        }
    }

    unsafe fn alloc_unchecked<T>(&self, value: T) -> &mut T {
        let ptr = (*self.handle.alloc_uninitialized(mem::size_of::<T>())).as_ptr() as *mut T;
        ptr::write(ptr, value);
        &mut *ptr
    }

    unsafe fn dynamic_drop<T>(&self, value: *mut T) {
        if mem::needs_drop::<T>() {
            self.items.borrow_mut().push(DynamicArenaItem {
                drop: mem::transmute::<unsafe fn(*mut T), unsafe fn(*mut c_void)>(
                    ptr::drop_in_place::<T>,
                ),
                value: value as *mut c_void,
            })
        }
    }

    pub fn alloc<T: 'a>(&self, value: T) -> &mut T {
        unsafe {
            let target = self.alloc_unchecked(value);
            self.dynamic_drop(target);
            target
        }
    }
}
