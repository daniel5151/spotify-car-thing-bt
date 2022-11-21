cfg_if::cfg_if! {
    if #[cfg(windows)] {
        mod win;
        pub use win::platform_init;
        pub use win::platform_teardown;
        pub use win::WinBtSockListener as BtSocketListener;
        pub use win::WinBtSockStream as BtSocketStream;
    } else {
        compile_error!("unsupported platform");
    }
}
