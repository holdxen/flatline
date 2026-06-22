use snafu::Snafu;

macro_rules! create_factory {
    ($ex:expr) => {
        Box::new(|| Box::new($ex) as _)
    };
}

// macro_rules! algo_list {
//     (
//         $all:ident,
//         $new_all:ident,
//         $new_by_name:ident,
//         $t:ty,
//         $($key:expr => $value:expr,)*
//     ) => {
//         pub fn $all() -> &'static [&'static str] {
//             &[
//                 $($key,)*
//             ]
//         }

//         pub fn $new_all() -> IndexMap<&'static str, Factory<$t>> {
//             let mut res: IndexMap<&'static str, Factory<$t>> = IndexMap::new();
//             $(
//                 res.insert($key,  Box::new(|| Box::new($value) as _));
//             )*
//             res
//         }

//         pub fn $new_by_name(name: &str) -> Option<Factory<$t>> {
//             match name {
//                 $($key => Some(Box::new(|| Box::new($value) as _)),)*
//                 _ => None,
//             }

//         }
//     }
// }

macro_rules! algo_list {
    (
        $all:ident,
        $new_all:ident,
        $new_by_name:ident,
        $t:ty,
        $(
            $(#[$cfg:meta])*
            $key:literal => $value:expr
        ),* $(,)?
    ) => {
        pub fn $all() -> &'static [&'static str] {
            &[
                $(
                    $(#[$cfg])*
                    $key,
                )*
            ]
        }

        pub fn $new_all() -> IndexMap<&'static str, Factory<$t>> {
            let mut res: IndexMap<&'static str, Factory<$t>> = IndexMap::new();

            $(
                $(#[$cfg])*
                res.insert($key, Box::new(|| Box::new($value) as _));
            )*

            res
        }

        pub fn $new_by_name(name: &str) -> Option<Factory<$t>> {
            match name {
                $(
                    $(#[$cfg])*
                    $key => Some(Box::new(|| Box::new($value) as _)),
                )*
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
pub mod signature;

#[derive(Debug, Snafu)]
pub enum Error {
    InvalidPrime,
    CompressError,
    MacVerificationFailed,
    MismatchKey,
    SignatureVerificationFailed,
    KeyLengthMismatch,
}

pub type Factory<T> = Box<dyn Fn() -> Box<T> + Send + Sync>;
