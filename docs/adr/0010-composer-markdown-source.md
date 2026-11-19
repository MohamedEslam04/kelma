# ADR 0010: The composer edits Markdown-style source, not rich text

- Status: accepted (2026-11-19)
- Date: 2026-11-19

## Context

Plan chat 5.4 asks for Teams' composer formatting: bold, italic, underline, strikethrough, font
color, highlight, size, headings, quotes, lists, code blocks with a language, tables and links.
Teams web edits rich text (CKEditor 5) and sends HTML.

kelma's UI is Slint (ADR 0001). Slint's `TextInput` is plain text. It has no inline styled runs,
no embedded blocks (a table, a code block) and no way to draw a selection over them. A WYSIWYG
editor would mean either writing a text editor widget (cursor, selection, IME, bidi, undo) or
embedding a web view for one text box.

## Options

| | Markdown-style source + preview | Own rich text editor widget | Web view editor |
|---|---|---|---|
| Arabic, bidi, IME, selection | the platform `TextInput` handles them | has to be rewritten, and is where bugs live | fine, but a second text stack |
| Cost | small: string edits plus a converter | large: weeks, and a permanent maintenance burden | a web view for every composer, memory and focus issues |
| Round trip | source to model to HTML, tested | full control | HTML out, needs a parser in |
| Looks like Teams while typing | no, markers are visible (a preview strip shows the result) | yes | yes |

## Decision

The composer stays a plain `TextInput` that holds Markdown-style source:

- `**bold**`, `*italic*`, `++underline++`, `~~strike~~`, `` `code` ``;
- `{color:#rrggbb}...{/}`, `{bg:#rrggbb}...{/}`, `{size:large|small}...{/}`;
- `# `, `## `, `### ` headings, `> ` quotes, `- ` and `1. ` lists (two spaces nest), `---` rules;
- fenced code with a language (` ```rust `), GFM pipe tables, `[text](url)`.

The toolbar and the Teams shortcuts do not edit rich text, they rewrite the source and the
selection (`render::edit::apply`, pure and unit tested). A **preview strip** under the toolbar
shows the message as it will look.

On send, `render::markdown_to_composer_html` turns the source into the HTML the Teams web
composer sends, through the `RichDoc` model (`render::rich`). The same model decodes incoming
HTML, so what is sent and what is received go through one code path. Every constructor
re-validates: colors are `#rrggbb` only, links are http, https or mailto, raw HTML in the source is
escaped.

Undo and redo: the input forgets its own stack when its text is set from code, so toolbar edits
keep their own history (`render::edit::History`). It is only used while the text is exactly what
the last toolbar edit left; after typing, the input's own undo runs.

## Consequences

- Plain messages are unchanged: text without markers converts to the same HTML as before (tested).
- Text that happens to look like Markdown is formatted: a line starting with `- ` becomes a list,
  `2*3*4` has an italic `3`. A backslash escapes a marker (`\*`).
- Editing a message that was sent with formatting shows its plain text, not the source it had.
  Turning a `RichDoc` back into source is a possible follow-up.
- Slint's `StyledText` has no headings, tables or background colors, so incoming rich messages
  draw a heading as a bold line, a table as aligned monospace rows and a highlight as a text
  color. A WYSIWYG widget would not change that for incoming messages.
- The cost of a real WYSIWYG editor can be revisited if Slint gains rich text editing.
