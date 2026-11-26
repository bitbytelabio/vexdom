# Site maps

A site map is a set of notes the agent writes from one exploration of a site. After you activate it,
every later side-panel run on that domain starts out knowing where search lives, what a button is
really called and why a list looks empty until you scroll, instead of spending its first minutes
rediscovering them.

---

## Making one

Press **Map this site** (or **Re-map this site**) in the side panel's **Skills** tab, or say:

```
@site-mapper map this site
```

Mapping needs the explicit `@site-mapper` prefix or the button. Trigger words alone will not start
one, because a mapping run takes minutes and drives the tab; asked any other way, the agent is told
to ask for it explicitly. It runs only in the side panel: an MCP client cannot start one.

Vexdom reads the site's own `robots.txt` and `sitemap.xml`, looks up public background on the
domain, then walks the site for a few minutes taking screenshots.

## What you get

```
~/vexdom/skills/acme-com/
├── SKILL.md          landmarks, key pages, how they connect, quirks
├── map.json          the structured report behind it
├── meta.json         host, domain and when it was made
├── screenshots/      captures taken during the crawl
├── evidence/         the robots.txt and sitemap it worked from
└── pages/            longer per-page notes, kept out of the prompt
```

The folder is `skillsDir` in `~/.vexdom/config.json` when you set one, else `~/vexdom/skills`. From
then on, every instruction you give on that domain carries those notes. Elsewhere they have no effect.

---

## Nothing takes effect until you say so

A map in progress is written to `~/vexdom/skills/.staging/<id>/`, which the skill loader **skips**
(it ignores dot folders), so an unreviewed map is never even opened, let alone used. Staged maps left
behind are swept away after a while.

The panel shows you the exact markdown as plain text, never rendered, with the domain it will match
and the folder it will go to. Tick **Only on ‹host›, not its subdomains** to narrow it. **Activate**
turns it on; **Discard** deletes it. The review appears on the conversation that mapped the site, so
switch back to that tab to decide. If that tab closes first, the review follows you, so an unsaved map
is never lost.

> A map is written from pages an agent read, so read it before activating, as you would any
> generated content. Its screenshots may show pages you were signed in to; they stay on this machine.

---

## What a mapping run may do

The crawl is **read-only and locked to one host**. It cannot click, fill or submit, and it cannot
leave the site. It is pinned to the tab it started in, so switching tabs stops it instead of taking
it with you. Off the host, every read is blocked until it navigates back.

Vexdom Bridge enforces the limits. Set them under `siteMap` in `~/.vexdom/config.json`; a value can
narrow a limit but never pass its ceiling:

| Key | Default | Ceiling |
| --- | --- | --- |
| `siteMap.maxPages` | 15 | 40 |
| `siteMap.maxScreenshots` | 10 | 24 |
| `siteMap.timeoutMs` | 600000 (10 min) | 1800000 (30 min) |

Two switches change what a run may do:

| Key | Default | Effect |
| --- | --- | --- |
| `siteMap.allowClicks` | `false` | Lets it reach routes that only exist behind an interaction |
| `siteMap.research` | `true` | Lets it use web search for public background on the domain |

`research` is the one case where a run both reads pages and makes outbound requests. Set it to
`false` to keep everything inside the browser.

```json
{ "siteMap": { "maxPages": 25, "research": false } }
```

---

## Writing notes by hand instead

To describe a site yourself, press **Upload notes** in the **Skills** tab and pick a markdown file:

```markdown
---
name: acme-admin
description: Our internal admin tool.
category: site-exploration
domains: [admin.acme.com]
---

Search is `#q` and submits on Enter, not on the button.
Results lazy load. Click "Load more" until it disappears before counting anything.
```

Notes are **overlays**: on a matching site they stack on top of whatever Vexdom was already
doing, so the normal driving and read-only rules still apply. Prefix an instruction with
`@acme-admin` to apply one wherever you are.

Notes live outside the repository and are re-read on every run, so an edit applies to the next thing
you ask. Hand-written notes come before generated ones in the prompt, and generated ones are labelled
machine-generated.

```sh
vexdom skills    # everything currently in scope, and where it came from
```

---

## See also

- [Skills](skills.md): how overlays and base skills fit together
- [Recordings](recordings.md): what you do on a site, where a site map describes the site itself
- [Configuration](../configuration.md): `~/.vexdom/config.json`
- [internals/subsystems.md](../../internals/subsystems.md): staging, validation, and the sweep
