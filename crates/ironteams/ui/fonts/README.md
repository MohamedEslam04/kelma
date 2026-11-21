# Bundled fonts

`TwemojiMozilla.ttf` is Twemoji in COLRv0 format, from
[mozilla/twemoji-colr](https://github.com/mozilla/twemoji-colr) v0.7.0.

- Graphics: Copyright Twitter, Inc. and other contributors, licensed under
  [CC-BY 4.0](https://creativecommons.org/licenses/by/4.0/).
- Font build tooling: Mozilla, Apache-2.0.

IronTeams bundles it because Slint's renderer supports COLRv0/CBDT color glyphs but not COLRv1, which
is the format most Linux distributions ship for their emoji font.
