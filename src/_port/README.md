# Staging area for un-ported eilmeldung code

This directory holds the original [eilmeldung](https://github.com/christo-auer/eilmeldung)
TUI modules. Nothing here is compiled — there are no `mod` declarations pointing
at it.

Phase 2 of the mitodo port moves modules out of here one at a time, rewriting
their RSS-specific parts against `crate::store`. When this directory is empty,
the port is done.

| module | becomes | phase |
|---|---|---|
| `ui/feeds_list` | `ui/groups_list` | 2 |
| `ui/articles_list` | `ui/items_list` | 2 |
| `ui/article_content` | `ui/item_detail` | 2 |
| `ui/chyron` | `ui/chyron` (vocabulary swap) | 3 |
| `query` | `query` (vocabulary swap) | 2 |
| `config`, `input`, `messages`, `utils` | same names | 2 |
