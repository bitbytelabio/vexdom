---
name: auth
description: Log in with an account the user saved in vexdom's encrypted vault (or their password manager plugin) without ever seeing the password, and list, show or delete saved accounts.
---

# Logging in with a saved account

The password never passes through you. The user saves an account once from a shell; you log in by
its name, and the Bridge fills the form inside the browser.

## Log in

`act {"do":"authList"}` lists the saved accounts (name, login URL, username — never passwords).
`act {"do":"authShow","name":"work"}` shows one.

`act {"do":"authLogin","name":"work"}` opens the account's login URL, waits for the username and
password fields, fills them, clicks submit and reports what happened. Then confirm you got in:
`waitFor` the post-login URL or a heading only a signed-in user sees.

- `noNavigate:true` logs in on the page already open — after a consent banner, a "Sign in" click or
  a challenge you cleared. The tab's origin must match the account's URL (or `url`, which then
  only constrains the origin).
- `usernameSelector`, `passwordSelector`, `submitSelector` override the fields for this login when
  the defaults pick the wrong ones; `timeoutMs` bounds the waits.
- `credentialProvider:"vault-plugin","item":"My App"` takes the account from a credential plugin
  the user configured instead of the vault.

Errors: `AUTH_PROFILE_NOT_FOUND` (check `authList`), `VAULT_KEY_MISSING` (the vault key is gone —
the user must restore `VEXDOM_VAULT_KEY` or `~/.vexdom/.vault-key`), `PLUGIN_FAILED`.

## What you must not do

- Saving an account is the user's job, never yours: it is not offered to you. If none exists, ask
  the user to run `vexdom auth save …` (below), or to log in themselves.
- Never ask the user to paste a password into the conversation.
- One-time codes (SMS, authenticator, email): ask the user for the code, then `fillInput` it.
- After a login, `act {"do":"state","op":"save","name":"work"}` keeps the session so later tasks
  can skip the login (the `page-state` playbook).
- `act {"do":"authDelete","name":"work"}` removes an account — only when the user asks.

Sealed placeholders (`⟦password:…⟧`) already on the page work too: pass one unchanged as a
`fillInput` value.

## From a shell (for the user)

`vexdom auth save <name> --url https://app.example.com/login --username ada@example.com --password-stdin`
(reads the password from stdin; `--password p` works but lands in shell history;
`--username-selector`, `--password-selector`, `--submit-selector` for unusual forms),
`vexdom auth login <name> [--no-navigate] [--url u]`, `vexdom auth list`, `vexdom auth show <name>`,
`vexdom auth delete <name>`. Credential plugins: `vexdom plugin add|list|show`.
