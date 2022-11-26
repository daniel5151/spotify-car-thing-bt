# spotify_carthing_bt

A grab-bag of spotify car thing related hacks I'm working on.

## deskthing-rs

A Rust port of <https://github.com/relative/deskthing>, which enables connecting
the car thing directly to a PC, and interfacing with the spotify client via a
custom [spicetify](https://spicetify.app/) extension.

At the time of writing, there is no prebuilt copy of the custom deskthing
spicetify extension, so you'll need to install node and manually build + install
the `spicetify_ext` extension.

Building the Rust code should be as easy as intalling Rust, cloning the repo,
and running:

```
cargo run
```

* * *

Currently windows only, but it shouldn't be _too_ hard to get it working on
linux by using a lib like
[bluer](https://docs.rs/bluer/0.15.1/bluer/index.html).

Contributions are welcome and appreciated!

## future projects

Connecting the stock firmware to host-side spotify is cool, sure, I'm far more
excited about the possibility of writing a custom carthing daemon + gui app to
do more exotic stuff with the hardware.

Expect to see some more code in this repo soon...
