#[cfg(test)]
mod tests {
    use crate::simple::*;

    #[test]
    fn t1() {
        let sys = System::new();
        let (d1_data, d1) = data_rel::new(sys.clone(), &[1, 2, 3]);
        let d2 = map_rel::new(sys.clone(), &d1, Box::new(|val| val + 1));
        let d3 = map_rel::new(sys.clone(), &d2, Box::new(|val| val / 2));
        let d3memo = memo_rel::new(sys.clone(), &d3);
        sys.run();
        assert_eq!(d2.clone().to_sorted_vec(), vec![2, 3, 4]);
        assert_eq!(d3.clone().to_sorted_vec(), vec![1, 1, 2]);
        assert_eq!(d3memo.clone().to_sorted_vec(), vec![1, 2]);
        d1_data.add(&5);
        println!("run//");
        sys.run();
        assert_eq!(d2.clone().to_sorted_vec(), vec![2, 3, 4, 6]);
        assert_eq!(d3.clone().to_sorted_vec(), vec![1, 1, 2, 3]);
        //assert_eq!(d3memo.clone().to_sorted_vec(), vec![1, 2, 3])
    }
}
