---
name: page-storage
description: Read or change what a site keeps in the browser — cookies (HttpOnly included), localStorage, sessionStorage — and use the system clipboard from the page.
---

# Cookies, web storage and the clipboard

## Web storage

`act {"do":"storage"}` returns every localStorage entry of the frame in focus;
`{"do":"storage","area":"session"}` reads sessionStorage. With `key` it reads one entry.
`op:"set"` (with `key` and `value`), `op:"remove"` (with `key`) and `op:"clear"` change it.
Many apps read storage only when they boot: after a change, `navigate {"action":"reload"}` and look
again.

## Cookies

`act {"do":"cookies"}` reads the cookies the browser would send to the tab's URL, HttpOnly ones
included; `url` reads another site's. Credential-shaped values (session ids, tokens) come back
sealed — you see that a cookie exists and when it expires, never its value.

`act {"do":"cookies","op":"set","name":"lang","value":"en","url":"https://example.com"}` sets
one; `domain`, `path`, `secure`, `httpOnly`, `sameSite` (`strict`, `lax`, `no_restriction`) and
`expires` (epoch seconds) are optional. `op:"clear"` removes one by `name`, or every cookie for the
URL without one. Clearing cookies logs the user out of that site: say so before you do it.

## Clipboard

`act {"do":"clipboard","op":"read"}` returns the clipboard's text; `op:"write"` with `text` puts
text on it; `op:"copy"` copies the page's current selection (select first with `selectText`);
`op:"paste"` pastes at the caret. The clipboard is the user's: read it only when the task is about
what they copied, and tell them when you overwrite it.

## Keeping a whole login

To save every cookie and storage entry of a tab and put them back later, use the `page-state`
playbook (`act {"do":"state","op":"save","name":"…"}`).

## From a shell

`vexdom storage local [key]`, `vexdom storage session set <k> <v>`, `vexdom storage local remove <k>`,
`vexdom storage local clear`; `vexdom cookies [get [name]]`,
`vexdom cookies set <name> <value> [--url u --domain d --path p --secure --http-only --same-site lax --expires <epoch s>]`,
`vexdom cookies set --curl <file>` (the cookies of a copied curl command), `vexdom cookies clear [name]`;
`vexdom clipboard read|write <text>|copy|paste`.
