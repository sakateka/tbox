# tbox

tbox keeps your small command-line utilities in one TOML config. Define an applet
and run it with `tbox NAME [args...]`. Add or change utilities by editing the config;
the binary does not need rebuilding.

Build and try the example:

```sh
cargo build --release --locked
./target/release/tbox --config tbox.example.toml --list
./target/release/tbox --config tbox.example.toml greet World
# Hello, World!
./target/release/tbox --config tbox.example.toml greet --help
```

Use [tbox.example.toml](tbox.example.toml) as a template for your applets.
Save your config to `~/.config/tbox/config.toml` to run without `--config`:

```sh
tbox --list
tbox NAME --help
tbox NAME [args...]
```

Put the built binary in a directory on your `PATH` to use `tbox` from anywhere.
