macro_rules! create_factory {
    ($ex:expr) => {
        Box::new(|| Box::new($ex) as _)
    };
}

macro_rules! algo_list {
    (
        $all:ident,
        $new_all:ident,
        $new_by_name:ident,
        $t:ty,
        $($key:expr => $value:expr,)*
    ) => {
        pub fn $all() -> &'static [&'static str] {
            &[
                $($key,)*
            ]
        }

        pub fn $new_all() -> IndexMap<&'static str, AlgoFactory<$t>> {
            let mut res: IndexMap<&'static str, AlgoFactory<$t>> = IndexMap::new();
            $(
                res.insert($key,  Box::new(|| Box::new($value) as _));
            )*
            res
        }

        pub fn $new_by_name(name: &str) -> Option<AlgoFactory<$t>> {
            match name {
                $($key => Some(Box::new(|| Box::new($value) as _)),)*
                _ => None,
            }

        }
    }
}

pub mod compress;
pub mod crypt;
pub mod hash;
pub mod kex;
pub mod mac;
pub mod sign;

/// 算法工厂：一个可重复调用的闭包，每次调用创建一个新的算法实例。
pub type AlgoFactory<T> = Box<dyn Fn() -> Box<T> + Send + Sync>;

// trait Backend {}
