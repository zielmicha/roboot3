#[cfg(test)]
mod tests {
    use crate::simple::*;

    #[test]
    fn t1() {
        with_system(|sys| {
            let d1 = data_rel(&sys, &[1, 2, 3]);
            let d2 = map(sys, d1.rel(), sys.alloc(&|val| val + 1));
            let d3 = map(sys, d2, sys.alloc(&|val| val / 2));
            let d3memo = memo(sys, d3);
            sys.run();
            assert_eq!(d2.to_sorted_vec(), vec![2, 3, 4]);
            assert_eq!(d3.to_sorted_vec(), vec![1, 1, 2]);
            assert_eq!(d3memo.to_sorted_vec(), vec![1, 2]);
        })
    }
}
