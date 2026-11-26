# Batch

`act {"batch":[…], "bail":true}` runs steps in order on one connection and answers with one line
per step: `1. clickElement ok: {…}` or `2. fillInput failed: CODE: message`.

Each step is an action with its input, the same object a single `act` takes:

    {"batch": [
      {"do": "fillInput", "target": "e7", "value": "Ada"},
      {"do": "fillInput", "target": {"label": "Email"}, "value": "ada@example.com"},
      {"do": "clickElement", "target": "e9"},
      {"do": "waitFor", "text": "Saved", "timeoutMs": 3000}
    ], "bail": true}

Without `bail` every step runs and the call reports which failed; with `bail` it stops at the first
failure. Refs follow the same rules as single actions: they must come from the latest snapshot of
the tab. Put only steps whose targets you already know in one batch; when a step's outcome decides
the next one, end the batch there and look.

## From a shell

`vexdom batch [--bail] [--json] [--omit-command]` takes steps as arguments
(`vexdom batch "fill @e7 Ada" "click @e9" "wait --text Saved"`), a JSON array of string arrays on
stdin (`[["fill","@e7","Ada"],["click","@e9"]]`), or one step per stdin line, either the CLI form
(`click @e9 --button right`) or JSON (`{"action":"page.fillInput","input":{"target":"e7","value":"Ada"}}`).
`--json` prints an array of `{command, success, result|error}`.
