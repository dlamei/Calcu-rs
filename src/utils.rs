use std::fmt;

pub(crate) type BuildHasher = fxhash::FxBuildHasher;
pub(crate) type HashMap<K, V, B = BuildHasher> = std::collections::HashMap<K, V, B>;
pub(crate) type HashSet<K, B = BuildHasher> = std::collections::HashSet<K, B>;
pub(crate) type Instant = quanta::Instant;

#[allow(unused_imports)]
pub(crate) mod log_macros {
    pub(crate) use log::{debug, error, info, log, trace, warn};
}

macro_rules! during_debug {
    () => {{
        #[cfg(debug_assertions)] true
        #[cfg(not(debug_assertions))] false
    }}
}

macro_rules! function_name {
    ($lvl: literal) => {{
        fn __f__() {}
        fn type_name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        type_name_of(__f__)
            .split("::")
            .skip($lvl)
            .filter(|&name| name != "__f__")
            .collect::<Vec<_>>()
            .join("::")

        //.find(|&part| part!= "f" && part != "{{closure}}")
        //.expect("function name")
    }};
}

macro_rules! trace_fn {
    () => {{
        use std::fmt::Write;
        let mut buff = String::new();
        write!(buff, "{}", crate::utils::function_name!(2)).unwrap();
        log::trace!("{}", buff);
    }};
    ($($tt:tt)+) => {{
        use std::fmt::Write;
        let mut buff = String::new();
        write!(buff, "{}: ", crate::utils::function_name!(2)).unwrap();
        write!(buff, $($tt)*).unwrap();
        log::trace!("{}", buff);
    }}
}

//pub trait Pow<Rhs = Self> {
//    type Output;
//    fn pow(self, rhs: Rhs) -> Self::Output;
//}

pub(crate) fn fmt_iter<E: fmt::Debug, F>(
    symbols: [&str; 3],
    mut it: impl Iterator<Item = E>,
    fmt_e: F,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result
where
    F: Fn(&E, &mut fmt::Formatter<'_>) -> fmt::Result,
{
    let start = symbols[0];
    let delimiter = symbols[1];
    let end = symbols[2];
    write!(f, "{start}")?;
    if let Some(first) = it.next() {
        fmt_e(&first, f)?;
    }
    for e in it {
        write!(f, "{delimiter}")?;
        fmt_e(&e, f)?;
    }
    write!(f, "{end}")?;
    Ok(())
}
