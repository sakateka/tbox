# tbox

tbox runs Starlark programs defined directly in its TOML configuration. No applets are bundled. Adding or editing a program takes effect on the next invocation with the same binary.

Build with `cargo build --release --locked`. Run `tbox --list` for available applets, `tbox NAME --help` for metadata and `tbox NAME [args...]` to run a script. A symlink to the binary named after an applet also works. The `[aliases]` table maps other executable names or direct subcommands to applet names.

Configuration comes from `--config PATH`, then `TBOX_CONFIG`, then `~/.config/tbox/config.toml`. Place the config option before the applet name. A missing default config is allowed; an explicitly selected missing config is an error. Paths resolve relative to the config file, and `~` expands to the home directory. See [tbox.example.toml](tbox.example.toml) for a complete runnable configuration with inline programs.

Executable aliases forward their arguments unchanged. Use `TBOX_CONFIG` to select a config when invoking an alias.

The applet list shows every configured alias with its applet's description, or the applet name when it has no aliases. Names are sorted alphabetically; the original applet name remains callable.

Define each applet as an `[applets.NAME]` table. Its `description`, `usage` and `script` fields are required and nonempty. The table key is the applet name; names contain ASCII letters, digits, `_` or `-`. Duplicate definitions are rejected by the TOML parser.

```toml
[applets.hello]
description = "Print a greeting."
usage = "hello <name>"
script = '''
def main(args):
    if len(args) != 1:
        fail("Usage: hello <name>")
    emit("Hello, " + args[0] + "!\n")
'''
```

Use TOML multiline literal strings (`'''`) so backslashes in Starlark code are preserved. No script directory or metadata files are needed. Try the complete example:

```sh
cargo build --locked
./target/debug/tbox --config tbox.example.toml --list
./target/debug/tbox --config tbox.example.toml greet World
# Hello, World!
```

Listing and help read metadata without parsing or evaluating Starlark. Syntax and runtime errors identify the config path, applet's script field and line within the program. Scripts are trusted local code and can invoke configured commands. Starlark uses its standard dialect and builtins, with these additional functions:

| Function | Behavior |
| --- | --- |
| `emit(text)` | Write text exactly, without adding a newline. |
| `http_request(profile, variables)` | Return UTF-8 response text from a configured HTTP request. |
| `render_url(profile, variables)` | Render a configured link without opening it. |
| `open_link(profile, variables)` | Render a link and pass one URL argument to its command binding. |
| `run_command(binding, args)` | Append argument strings to a configured command; inherit standard I/O. |
| `relative_path(root)` | Return cwd relative to a configured root; reject cwd outside it. |
| `random_string(length, alphabet)` | Sample characters uniformly using OS cryptographic randomness; length 0–1000000. |
| `setting(name)` | Read a string from `[settings]`. |

Commands are argument arrays and run directly without a shell. A relative executable containing `/` resolves against the config directory. Command failure makes the applet fail. Python expressions can be delegated through a configured Python command when Python semantics are needed.

HTTP profiles support a URL template, method, literal body, headers, token reference, authorization scheme, PEM CA bundle and timeout. Token references are read only when that profile runs; environment values and plain token files are supported. Authorization defaults to `OAuth`; credential acquisition and Basic authentication are unsupported. Token values must contain printable ASCII without whitespace; a token file may end with a newline. Errors do not print tokens or request URLs. TLS certificate and hostname validation remain enabled; configured CAs supplement normal roots. Redirects are disabled and only successful status codes return a body. Timeouts default to 30 seconds and must be 1–3600 seconds.

URL placeholders such as `{value}` encode a single path, query or fragment component, including spaces, slashes, percent signs and delimiters. `{path:path}` preserves `/` between path segments and is allowed only in the URL path. Template scheme and authority must be literal HTTP(S), without embedded credentials. Missing profiles or variables fail before opening a process.

Path substitutions reject dot-only segments (`.` and `..`) so URL normalization cannot change the surrounding path. Command-line arguments must be valid UTF-8; invalid arguments return an error.

Keep real utility programs, credentials and internal settings together in an ignored local config. The repository ignores `/tbox.toml`; public examples and tests use synthetic programs and local fixtures only.

Verify with `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo fmt --all -- --check`.

For Neovim with Treesitter, [the injection query](contrib/nvim/queries/toml/injections.scm) treats multiline literal `script` fields inside `[applets.NAME]` as Starlark. TOML itself has no language tag; the query recognizes the field, so a language comment is unnecessary. Both `toml` and `starlark` parsers must be available. With nvim-treesitter, install missing parsers using `:TSInstall toml starlark`.

Copy the query to `<Neovim config>/after/queries/toml/injections.scm` (merge it if that file already exists), then reopen the config. Find that directory with `:lua print(vim.fn.stdpath('config'))`; its default is `~/.config/nvim`. `:InspectTree` can show the injected Starlark tree. This uses Neovim's [language injections](https://neovim.io/doc/user/treesitter/#treesitter-language-injections).
