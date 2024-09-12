/// counter_bundle declares a struct holding a bundle of counters
/// that should be incremented or decremented together. This
/// is used to facilitate computing rolled up metrics.
#[macro_export]
macro_rules! counter_bundle {
    (pub struct $name:ident {
        $(
            pub $fieldname:ident: AtomicCounter,
        )*
    }
    ) => {
            #[derive(Clone)]
            pub struct $name {
                $(
                    pub $fieldname: AtomicCounter,
                )*
            }

            impl $name {
                pub fn inc(&self) {
                    $(
                        self.$fieldname.inc();
                    )*
                }
                pub fn dec(&self) {
                    $(
                        self.$fieldname.dec();
                    )*
                }
                pub fn sub(&self, n: usize) {
                    $(
                        self.$fieldname.sub(n);
                    )*
                }
                pub fn inc_by(&self, n: usize) {
                    $(
                        self.$fieldname.inc_by(n);
                    )*
                }
            }
    };
}
