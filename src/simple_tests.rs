#[cfg(test)]
mod tests {
    use crate::simple::*;

    #[test]
    fn exploration() {
        let sys = System::new();
        let _d1 = data_rel(sys, &[1, 2, 3]);
    }
}
