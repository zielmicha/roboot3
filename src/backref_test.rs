#[cfg(test)]
mod tests {
    use crate::backref::*;

    fn on_remove(this: R<i64>, value: i64) {
        print!("on_remove {} {}\n", *this, value);
    }

    #[test]
    fn t1() {
        let a0 = R::new(Box::new(6));
        {
            let this1 = R::new(Box::new(1));
            let expando1 = Expando::new(this1.clone(), &on_remove);
            let a2 = R::new(Box::new(2));
            expando1.add(a2.clone(), 12);
            expando1.add(a0.clone(), 10);
            {
                let a1 = R::new(Box::new(1));
                expando1.add(a1, 11);
            }
        }
    }
}
