# The Bridge wire (ADR-025): every WebSocket message between the extension and the daemon, and
# between the CLI/MCP server and the daemon, is a binary frame of this schema's messages as an
# unaligned byte stream. Documents owned and validated by TypeScript (`src/lib`, zod) travel as
# `Json` text; everything protocol-level is typed here.
#
# Conventions: an empty Text means "absent" for optional strings; `tabId = -1` means no tab;
# `depth = -1` means no depth. Ordinals are in declaration order.
@0x9f4fbb73b5296b17;

using Json = Text;

struct ActionError {
  code @0 :Text;
  message @1 :Text;
}

struct ActionResult {
  union {
    ok @0 :Json;
    error @1 :ActionError;
  }
}

# ---------------------------------------------------------------------------------------------
# Extension socket: the bootstrap capability is ExtensionGate.

interface ExtensionGate {
  # A refused hello is a capnp exception whose text is the reason (TS close reasons).
  hello @0 (protocolVersion :UInt32, minDaemonProtocol :UInt32, extensionVersion :Text,
            manifestHash :Text, auth :AuthKind, nonce :Text, installId :Text, browser :Text)
        -> (serverNonce :Text);
  prove @1 (proof :Text, page :Page, ui :ExtensionUi) -> (outcome :ProveOutcome);
}

enum AuthKind {
  pair @0;
  session @1;
}

struct ProveOutcome {
  union {
    welcome @0 :Welcome;
    unauthorized @1 :Unauthorized;
  }
}

struct Welcome {
  daemonVersion @0 :Text;
  protocolVersion @1 :UInt32;
  manifestHash @2 :Text;
  manifestInSync @3 :Bool;
  proof @4 :Text;
  sealedSessionKey @5 :Text;
  session @6 :ExtensionSession;
}

struct Unauthorized {
  reason @0 :Text;
  retryable @1 :Bool;
  code @2 :Text;   # "bad-code" | "unknown" | "revoked" | ""
}

# Served by the extension, called by the daemon.
interface Page {
  describe @0 () -> (tools :Json);
  invoke @1 (action :Text, input :Json, tabId :Int64, runId :Text) -> (result :ActionResult);
  # `page.screenshot`: the image travels as bytes; the result JSON carries no `dataUrl`.
  screenshot @2 (input :Json, tabId :Int64) -> (result :ActionResult, image :Data);
  runTask @3 (order :Json) -> (result :ActionResult);
  # A file the Bridge writes for the user (PDF, HAR, trace): its bytes travel as `data`.
  artifact @4 (action :Text, input :Json, tabId :Int64) -> (result :ActionResult, data :Data);
}

# Daemon → extension pushes.
interface ExtensionUi {
  run @0 (id :Text, event :RunEvent);
  siteMapDraft @1 (id :Text, draft :Json);
  openNotice @2 (noticeId :Text);
  agentInfo @3 (result :ActionResult);
  skillCatalog @4 (result :ActionResult);
  preferencesInfo @5 (result :ActionResult);
  taskList @6 (result :ActionResult);
}

# Extension → daemon requests, one method per TS request frame.
interface ExtensionSession {
  ping @0 ();
  focus @1 ();
  instruct @2 (id :Text, text :Text, context :Json);
  cancel @3 (id :Text);
  decision @4 (id :Text, toolId :Text, allow :Bool, remember :Bool);
  reset @5 (sessionId :Text);
  analyzeFile @6 (id :Text, fileId :Text, sessionId :Text, name :Text, mime :Text, size :UInt64,
                  content :Data) -> (result :ActionResult);
  cancelAnalysis @7 (id :Text, fileId :Text);
  saveSkill @8 (skill :Json) -> (result :ActionResult);
  deleteSkill @9 (name :Text) -> (result :ActionResult);
  deleteSiteMap @10 (name :Text) -> (result :ActionResult);
  activateSiteMap @11 (stagingId :Text, exactHost :Bool) -> (result :ActionResult);
  discardSiteMap @12 (stagingId :Text) -> (result :ActionResult);
  nameSession @13 (host :Text, messages :List(Text)) -> (result :ActionResult);
  analyzeRecording @14 (recording :Json) -> (result :ActionResult);
  agentState @15 (refresh :Bool) -> (result :ActionResult);
  setAgent @16 (agent :Text) -> (result :ActionResult);
  setAgentModel @17 (agent :Text, model :Text) -> (result :ActionResult);   # "" = null
  grantAgent @18 (agent :Text) -> (result :ActionResult);
  listSkills @19 (refresh :Bool) -> (result :ActionResult);
  preferences @20 () -> (result :ActionResult);
  setPreference @21 (change :Json) -> (result :ActionResult);
  tasks @22 () -> (result :ActionResult);
  saveTask @23 (task :Json) -> (result :ActionResult);
  deleteTask @24 (taskId :Text) -> (result :ActionResult);
  runTaskNow @25 (taskId :Text) -> (result :ActionResult);
  pauseTasks @26 (paused :Bool) -> (result :ActionResult);
  taskDone @27 (taskId :Text, result :Json);
  notice @28 (notice :Json) -> (delivered :Bool);
  # One screencast frame (JPEG) of a `page.recordStart` recording; the Bridge encodes them.
  recordFrame @29 (recordingId :Text, frame :Data, atMs :Float64);
  # The standing "Always on this site" approvals, and forgetting them ("" = all of them). From 28.
  approvals @30 () -> (result :ActionResult);
  forgetApprovals @31 (host :Text) -> (result :ActionResult);
}

struct RunEvent {
  union {
    started :group {
      skill @0 :Text;
      attached @1 :Json;
      overlays @2 :Json;
    }
    text @3 :Text;
    tool :group {
      toolId @4 :Text;
      action @5 :Text;
      input @6 :Json;
      source @7 :Text;
    }
    approval :group {
      toolId @8 :Text;
      action @9 :Text;
      input @10 :Json;
      site @11 :Text;
    }
    toolResult :group {
      toolId @12 :Text;
      ok @13 :Bool;
      summary @14 :Text;
    }
    session :group {
      agent @15 :Text;
      agentSessionId @16 :Text;
    }
    attachments @17 :Json;
    usage :group {
      contextTokens @18 :UInt64;
      outputTokens @19 :UInt64;
    }
    done @20 :Text;   # stopReason
    error @21 :ActionError;
  }
}

# ---------------------------------------------------------------------------------------------
# Control socket: the bootstrap capability is Control (the role is decided at the HTTP upgrade by
# the Bearer token). A control connection that calls `describe` or `invoke` with a non-empty
# `runId` speaks for that agent run from then on.

interface Control {
  status @0 () -> (status :BridgeStatus);
  describe @1 (runId :Text) -> (tools :Json, reserved :List(Text), withheld :List(Text));
  # Cancel = drop the call (capnp Finish).
  invoke @2 (action :Text, input :Json, runId :Text) -> (result :ActionResult, image :Data);
  pair @3 () -> (code :Text, expiresAt :Int64);
  sessions @4 () -> (sessions :List(SessionSummary));
  agent @5 (set :Text, grant :Text, models :Text) -> (state :Json);
  preferences @6 () -> (result :ActionResult);
  setPreference @7 (change :Json) -> (result :ActionResult);
  grant @8 (installId :Text, caller :Text, browser :Text, extensionVersion :Text, gesture :Bool,
            waitMs :UInt32) -> (outcome :Json);
  pairRequests @9 () -> (requests :Json);
  decide @10 (request :Text, allow :Bool, label :Text) -> (ok :Bool, error :Text);
  pairing @11 (set :Text) -> (mode :Text, approver :Text, error :Text);
  blocked @12 () -> (blocked :Json);
  label @13 (session :Text, label :Text) -> (ok :Bool, error :Text);
  takeNotices @14 () -> (notices :Json);
  openNotice @15 (notice :Text) -> (ok :Bool);
  revoke @16 (session :Text, origin :Text) -> (revoked :UInt32);
  # Drop the handle to stop watching. `approver` names a pair-request approver ("app" or
  # "terminal"); `allowed` is the notices watcher's "this computer lets me show notices".
  watch @17 (topic :Topic, approver :Text, allowed :Bool, sink :EventSink) -> (handle :Handle);
  snapshot @18 (opts :SnapshotOpts) -> (text :Text, webmcp :Text);
  # `webmcp`: JSON, the WebMCP context when the tab's tools changed since the last announcement; empty otherwise.
  read @19 (opts :ReadOpts) -> (text :Text);
  # Graceful shutdown, used by `vexdom stop` before it falls back to signals.
  stop @20 ();
  # External confirmations (ADR-032): actions from MCP clients and the CLI waiting for the user.
  # `list`: [{id, action, summary, site, input, approver, createdAt}]. Refused to agent runs.
  confirmations @21 () -> (list :Json);
  confirm @22 (id :Text, allow :Bool, remember :Bool) -> (found :Bool);
}

enum Topic {
  manifestChanged @0;
  pairRequestsChanged @1;
  noticesChanged @2;
  settingsChanged @3;
  confirmationsChanged @4;   # payload: the pending external confirmations, as `confirmations` lists them
}

interface EventSink {
  event @0 (topic :Topic, payload :Json);
}

interface Handle {}

struct SnapshotOpts {
  interactive @0 :Bool;
  compact @1 :Bool;
  depth @2 :Int32;
  selector @3 :Text;
  urls @4 :Bool;
  full @5 :Bool;
  tabId @6 :Int64;
  runId @7 :Text;
  # The reply as JSON: {"text", "refs": [{"ref", "role", "name"}]}.
  json @8 :Bool;
}

struct ReadOpts {
  outline @0 :Bool;
  filter @1 :Text;
  maxChars @2 :UInt32;
  target @3 :Json;
  runId @4 :Text;
  offset @5 :UInt32;
}

# `control.ts` BridgeStatus, in order.
struct BridgeStatus {
  connected @0 :Bool;
  daemonVersion @1 :Text;
  protocolVersion @2 :UInt32;
  port @3 :UInt16;
  manifestInSync @4 :Bool;
  extensionVersion @5 :Text;
  browser @6 :Text;
  connectedBrowsers @7 :UInt32;
  pairedBrowsers @8 :UInt32;
  pairingPending @9 :Bool;
  pairing :group {
    mode @10 :Text;
    approver @11 :Text;
  }
}

# `control.ts` SessionSummary, in order.
struct SessionSummary {
  id @0 :Text;
  browser @1 :Text;
  origin @2 :Text;
  source @3 :Text;
  extensionVersion @4 :Text;
  pairedAt @5 :Text;
  lastSeenAt @6 :Text;
  connected @7 :Bool;
  label @8 :Text;
  pairedBy @9 :Text;
}
