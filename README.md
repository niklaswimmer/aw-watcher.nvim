# aw-watcher.nvim

An [ActivityWatch](https://activitywatch.net) watcher implementation for Neovim, written in Rust.

## Project Status

**Status**: POC

Currently, this plugin is a 1-to-1 port of the official watcher implementation for Vim (which is written in Vim-Script) - [check it out](https://github.com/activitywatch/aw-watcher-vim).

No further features are added, if anything, features are missing (see comparison below).

I have some plans on how I could expand the plugins functionality (which I guess would make it "better" than the official one), but depending on my free time it may as well just stay like this - a fun little experiment.

## Comparison with the offical Vim watcher

I created the project mainly as an excuse to write some more Rust code, so I have not put much brain power into the advantages and downsides this plugin would have compared to the already existing `aw-watcher-vim`. Nevertheless, here are some pros and cons I discovered during and after writing this plugin (I may expand upon that list in the future, and it is for sure not exhaustive).

### Pros

- typesafe interaction with Neovim's API via `nvim-oxi`
- access to the Rust crate ecosystem
  - this for example allows us to use the `aw-client-rust` and `aw-models` crates for this plugin, which simplifies interacting with the ActivityWatch server, compared to manually crafting URLs and specifying URL parameters via strings (or creating custom models)
- doing the networking in the background is simpler IMO, as we can just spawn another thread to do that work
  - the Vim-Script version uses `start_job` and `startjob` on Vim and Neovim respectively, which has the same effect I think
- by specifically targetting Neovim, we can make use of its features and APIs
  - until now nothing is done which is not as well easily achievable with Vim-Script in standard Vim, but I like the idea that this can be changed in the future

### Cons

- the Vim-Script version is more approachable
  - a Rust toolchain has to be installed for developing and installing this plugin (or atleast providing a good installation procedure is much harder on my end)
  - Vim-Script is just simpler than Rust
- the installation process is more complicated
- Rust is much more verbose, which might not be needed for such a simple plugin
  - in fact, I have reason to believe that this verboseness slows the plugin down quite much when compared to Vim-Script
  - the Vim-Script version can calculate the current file, project and language everytime a heartbeat event is triggered - in the Rust version this has to be delayed after the One-Second check as otherwise the plugin is too slow
  - my guess is that this is due to the extra error handling and everything that is needed in Rust when compared to Vim-Script, I would like to do some actual benchmarks and timings in the future but for now my uneducated guess has to be enough

