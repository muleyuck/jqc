# Changelog

## [0.2.0](https://github.com/muleyuck/jqc/compare/v0.1.0...v0.2.0) (2026-07-17)


### ⚠ BREAKING CHANGES

* replace set/del/push subcommands with jq assignment operator syntax ([#19](https://github.com/muleyuck/jqc/issues/19))

### Features

* add --null-input / -n flag to run filters without reading input ([f2d3c8a](https://github.com/muleyuck/jqc/commit/f2d3c8a2d402ddc8b28843ede2789b18e4cc0182))
* add --null-input / -n flag to run filters without reading input ([e1a961b](https://github.com/muleyuck/jqc/commit/e1a961bc360a38e08fc5e8ca6d7488ece0d41fcd))
* allow `set` to create a new key if it doesn't already exist ([#18](https://github.com/muleyuck/jqc/issues/18)) ([b889fa0](https://github.com/muleyuck/jqc/commit/b889fa0e41cd3dff99bdd39ae92727533817802d))
* replace set/del/push subcommands with jq assignment operator syntax ([#19](https://github.com/muleyuck/jqc/issues/19)) ([a49ae03](https://github.com/muleyuck/jqc/commit/a49ae0370b8a4f063df74ae0bfdc8e17c1b2bcb7))

## [0.1.0](https://github.com/muleyuck/jqc/releases/tag/jqc-v0.1.0) (2026-05-06)


### Features

* add ANSI color output with NO_COLOR and JQ_COLORS support ([be052fe](https://github.com/muleyuck/jqc/commit/be052fe7078ffd28150b16590baa80ffa8325a7f))
* add CST navigation using path segments with edge case tests ([2f9a54b](https://github.com/muleyuck/jqc/commit/2f9a54b228331aefc76a4bc17b59bd938a4a97f8))
* add fmt subcommand and ANSI color output for all modes ([f7f2061](https://github.com/muleyuck/jqc/commit/f7f2061736f54af1db7d2e18513c60bc053f7bde))
* add jq-compatible filter execution engine using jaq-core ([6628853](https://github.com/muleyuck/jqc/commit/662885327c5c5f272e02e4c9fea1bd568daa5ed5))
* add JSONC parser wrapper and module structure ([5055956](https://github.com/muleyuck/jqc/commit/5055956731b4f022337b1e9674efe6512ce204c8))
* add set/del/push subcommands with in-place editing support ([f4f963d](https://github.com/muleyuck/jqc/commit/f4f963d2ea63650b70dfb928dae33500c228113e))
* implement CLI entry point with filter execution and output formatting ([a02504a](https://github.com/muleyuck/jqc/commit/a02504a073cff31e328ed6e85b64684bd8752518))
* implement del operation with comment-preserving CST mutation ([81c6657](https://github.com/muleyuck/jqc/commit/81c6657356493647c76a47a8afb579383055b1be))
* implement push operation with comment-preserving CST mutation ([6d55061](https://github.com/muleyuck/jqc/commit/6d55061de521038a1f318c2741c4c87b42244ac8))
* implement set operation with comment-preserving CST mutation ([a831262](https://github.com/muleyuck/jqc/commit/a831262df46e0ed87711a6b557a07a17d1d7dca6))


### Bug Fixes

* default to identity filter "." when no filter argument is given ([21c582f](https://github.com/muleyuck/jqc/commit/21c582fe9677c56d73905ad3e8bc5e569e75d437))
* improve filter error messages with human-readable formatting ([8c7e608](https://github.com/muleyuck/jqc/commit/8c7e608a6fa3a590e5bf7615ba9ee1e36d8595c6))
