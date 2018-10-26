# xi-term

[![Build Status](https://travis-ci.org/xi-frontend/xi-term.svg?branch=master)](https://travis-ci.org/xi-frontend/xi-term)

Formerly `xi-tui`, `xi-term` is a terminal frontend for [xi](https://github.com/xi-editor/xi-editor/).

It is experimental and under development, so don't expect anything magical (yet!).

## Installation

The frontend assumes that you have installed the
[core editor](https://github.com/xi-editor/xi-editor)
and is available in your PATH. The following should suffice:

```bash
git clone https://github.com/xi-editor/xi-editor
cd xi-editor/rust
cargo install

# if you want syntax highlighting, you need to install the syntect plugin:
cd syntect-plugin
make install

# You need to add ~/.cargo/bin to your PATH
# (this is where `cargo install` places binaries).
# In your .bashrc (or equivalent), add `export PATH=$PATH:~/.cargo/bin`
```

Then you can clone this repository and run the frontend with
`cargo run --release -- <your_file>`.
`your_file` can be an existing file or any dummy name.

## Logging

For debugging, it can be useful to have logs.
You can specify a location for log files `xi-term` with `-l <logfile>`.
Two files will be written:

- `<logfile>`: all the `xi-term` logs
- `<logfile>.rpc`: the RPC messages exchanged between the core and the frontend

## Screenshots

![a python file](.github/python.png)

![the README file](.github/README.png)

## Shortcuts

For now, there are only two shortcuts:

- `^w` saves the current view
- `^c` exits

## Commands
`xi-term` supports a vim-like command prompt for executing commands, accessed
via Alt-X.

Currently supported commands:

| Short form | Long form | Description |
| ---------- | --------- | ----------- |
| q | quit | Quits xi-term |
| s | save | Saves the current file |
| o `filename` | open `filename` | Open `filename` for editing |
| b | backspace | Delete the previous character and move the cursor one position back |
| d | delete | Delete the character under the cursor |
| bn | next-buffer | Switch to the next buffer |
| bp | prev-buffer | Switch to the previous buffer |
| pd | page-down | Advance the current view by one page |
| pu | page-up | Move the current view back by one page |
| t `theme` | theme `theme-name` | Set the theme to `theme`|

Future commands:

| Short form | Long form | Description |
| ---------- | --------- | ----------- |
| c | close | Closes the current view |
| ? `string` | search `string` | Search for `string` |
| ml | move-left | Move the cursor one position left |
| mr | move-right | Move the cursor one position right |
| mu | move-up | Move the cursor one line up |
| md | move-down | Move the cursor one line down |
| sl | select-left | Move the cursor one position left and update the current selection accordingly |
| sr | select-right | Move the cursor one position right and update the current selection accordingly |
| su | select-up | Move the cursor one line up and update the current selection accordingly |
| sd | select-down | Move the cursor one line down and update the current selection accordingly |

## Preferences
Xi-core supports several user-configurable options through a `preferences.xiconfig` file.
The default location for this is `$XDG_CONFIG_HOME/xi/preferences.xiconfig`, or, if
`$XDG_CONFIG_HOME` is not set, it defaults to `$HOME/xi/preferences.xiconfig`.

## Caveats

### Tabs

We assume at least one of the following is true:

1. the `translate_tabs_to_spaces` config option is enabled, or
2. the `tab_size` config option matches your terminal tabstop settings.

The xi-core options may be specified in your preferences.xiconfig file. For
most filetypes, `translate_tabs_to_spaces` defaults to 'on' (although it is
disabled for Makefiles).  The default `tab_size` is 4.

Note that many terminals default to a tab size of 8. You can change your tab
size to 4 with the command:

```
tabs -4
```

or update your user config `tab_size` to 8:

`preferences.xiconfig`:
```
tab_size = 8
```

### Colors

If you have the `syntect` plugin installed, colors will be enabled by default, with two caveats:

- you must have true colors enabled. Otherwise, some portions of text won't be displayed
- the default theme is for dark backgrounds
