#[cfg(test)]
mod tests {
    use crate::simple::*;

    #[test]
    fn t1() {
        with_system(|sys| {
            let d1 = data_rel(&sys, &[1, 2, 3]);
            let f = &|val| val + 1;
            let d2 = map(sys, d1.rel(), sys.alloc(f));
            d2.meta.iter_values(&|val| print!("val {}\n", val));
        })
    }
}
