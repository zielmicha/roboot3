pub trait Row: std::hash::Hash + Eq + Clone + 'static {}

impl<T> Row for T where T: std::hash::Hash + Eq + Clone + 'static {}
