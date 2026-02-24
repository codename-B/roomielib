//! Debug output enabled by the `debug` Cargo feature.

#[cfg(feature = "debug")]
#[macro_export]
macro_rules! debug_println {
    ($($arg:tt)*) => { eprintln!("[roomie] {}", format!($($arg)*)) };
}

#[cfg(not(feature = "debug"))]
#[macro_export]
macro_rules! debug_println {
    ($($arg:tt)*) => {};
}
