//! The extension handshake as the socket's bootstrap capability (`daemon.ts:318-410`): `hello`
//! then `prove`, each within 10 s; a pairing code or a session key proves the browser, and the
//! welcome carries the `ExtensionSession` capability the extension calls from then on.

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use futures::{AsyncRead, AsyncWrite};
use vexdom_core::handshake::new_nonce;
use vexdom_core::proto::{MIN_EXTENSION_PROTOCOL, SOCKET_PROTOCOL_VERSION, is_install_id};
use vexdom_core::{auth_store, log, tools};
use vexdom_pure::handshake::{self, Transcript};
use vexdom_schema::{AuthKind, extension_gate, extension_session};

use crate::Daemon;
use crate::link::{Conn, Hello, Link};
use crate::wire::text;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const REFUSAL_GRACE: Duration = Duration::from_millis(100);
const BROWSER_LABEL_MAX: usize = 40;

/// Serves one extension socket with a fresh gate.
pub(crate) fn accept<T>(daemon: &Daemon, stream: T, origin: String)
where
    T: AsyncRead + AsyncWrite + Unpin + 'static,
{
    let conn = Rc::new(Conn::default());
    let gate = Rc::new(Gate {
        daemon: daemon.clone(),
        conn: Rc::clone(&conn),
        origin,
        state: RefCell::new(State::Waiting),
        link: RefCell::new(None),
        generation: RefCell::new(0),
    });
    let client: extension_gate::Client = capnp_rpc::new_client_from_rc(Rc::clone(&gate));
    let system = vexdom_link::serve(stream, client.client);
    let watched = Rc::clone(&gate);
    let task = tokio::task::spawn_local(async move {
        let _ = system.await;
        watched.ended("socket closed");
    });
    *conn.abort.borrow_mut() = Some(task.abort_handle());
    gate.arm_timeout();
}

enum State {
    Waiting,
    Helloed(Box<(Hello, Transcript, AuthKind)>),
    Proving,
    Done,
}

struct Gate {
    daemon: Daemon,
    conn: Rc<Conn>,
    origin: String,
    state: RefCell<State>,
    link: RefCell<Option<Rc<Link>>>,
    generation: RefCell<u64>,
}

/// `browserLabel`: printable ASCII, trimmed, at most 40 characters.
fn browser_label(claimed: &str) -> Option<String> {
    let printable: String = claimed
        .chars()
        .filter(|c| (' '..='~').contains(c))
        .collect();
    let label: String = printable.trim().chars().take(BROWSER_LABEL_MAX).collect();
    (!label.is_empty()).then_some(label)
}

impl Gate {
    /// The next frame must come within 10 s, or the socket goes.
    fn arm_timeout(self: &Rc<Self>) {
        let generation = {
            let mut generation = self.generation.borrow_mut();
            *generation += 1;
            *generation
        };
        let gate = Rc::clone(self);
        tokio::task::spawn_local(async move {
            tokio::time::sleep(HANDSHAKE_TIMEOUT).await;
            let current = *gate.generation.borrow();
            if current == generation && !matches!(*gate.state.borrow(), State::Done) {
                log::log("extension handshake timed out");
                gate.conn.close();
            }
        });
    }

    /// Closes the socket a moment after the refusal went out.
    fn close_soon(&self) {
        *self.state.borrow_mut() = State::Done;
        let conn = Rc::clone(&self.conn);
        tokio::task::spawn_local(async move {
            tokio::time::sleep(REFUSAL_GRACE).await;
            conn.close();
        });
    }

    fn refuse<T>(&self, reason: String) -> Result<T, capnp::Error> {
        log::log(&format!("rejected extension: {reason}"));
        self.close_soon();
        Err(capnp::Error::failed(reason))
    }

    fn ended(&self, reason: &str) {
        if let Some(link) = self.link.borrow_mut().take() {
            link.dispose(reason);
        }
    }
}

impl extension_gate::Server for Gate {
    fn hello(
        self: capnp::capability::Rc<Self>,
        params: extension_gate::HelloParams,
        mut results: extension_gate::HelloResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let outcome = (|| {
            let params = params.get()?;
            if !matches!(*self.state.borrow(), State::Waiting) {
                return self.refuse("expected a proof".into());
            }
            let protocol_version = params.get_protocol_version();
            if protocol_version < MIN_EXTENSION_PROTOCOL {
                return self.refuse(format!(
                    "protocol version mismatch: Vexdom Bridge speaks v{SOCKET_PROTOCOL_VERSION} and takes extensions from v{MIN_EXTENSION_PROTOCOL} on"
                ));
            }
            let min_daemon = params.get_min_daemon_protocol();
            if min_daemon > SOCKET_PROTOCOL_VERSION {
                return self.refuse(format!(
                    "daemon too old: Vexdom Bridge speaks v{SOCKET_PROTOCOL_VERSION}, this extension needs v{min_daemon}"
                ));
            }
            let nonce = text(params.get_nonce());
            if !handshake::is_nonce(&nonce) {
                return self.refuse("expected a nonce".into());
            }
            let install_id = text(params.get_install_id());
            if !is_install_id(&install_id) {
                return self.refuse("expected an install id".into());
            }
            let extension_version = text(params.get_extension_version());
            let manifest_hash = text(params.get_manifest_hash());
            let auth = params
                .get_auth()
                .map_err(|_| capnp::Error::failed("expected a credential".into()))?;
            let server_nonce = new_nonce();
            let transcript = Transcript {
                protocol_version,
                extension_version: extension_version.clone(),
                manifest_hash: manifest_hash.clone(),
                client_nonce: nonce,
                server_nonce: server_nonce.clone(),
            };
            let hello = Hello {
                install_id,
                origin: self.origin.clone(),
                extension_version,
                manifest_hash,
                browser: browser_label(&text(params.get_browser())),
                protocol_version,
            };
            *self.state.borrow_mut() = State::Helloed(Box::new((hello, transcript, auth)));
            results.get().set_server_nonce(&server_nonce);
            Ok(())
        })();
        if outcome.is_ok() {
            self.arm_timeout();
        }
        std::future::ready(outcome)
    }

    fn prove(
        self: capnp::capability::Rc<Self>,
        params: extension_gate::ProveParams,
        mut results: extension_gate::ProveResults,
    ) -> impl Future<Output = Result<(), capnp::Error>> + 'static {
        let gate = Rc::clone(&self);
        let started = (|| {
            let params = params.get()?;
            let state = std::mem::replace(&mut *gate.state.borrow_mut(), State::Proving);
            let State::Helloed(helloed) = state else {
                return gate.refuse("expected hello".into());
            };
            let (hello, transcript, auth) = *helloed;
            Ok((
                text(params.get_proof()),
                params.get_page()?,
                params.get_ui()?,
                hello,
                transcript,
                auth,
            ))
        })();
        async move {
            let (proof, page, ui, hello, transcript, auth) = started?;
            let install = auth_store::Install {
                install_id: hello.install_id.clone(),
                origin: hello.origin.clone(),
                extension_version: hello.extension_version.clone(),
                browser: hello.browser.clone(),
            };
            let proven = match auth {
                AuthKind::Pair => prove_pairing(&gate.daemon, &proof, &transcript, &install).await,
                AuthKind::Session => prove_session(&proof, &transcript, &install).await,
            };
            let (secret, sealed) = match proven {
                Ok(found) => found,
                Err(refusal) => {
                    log::log(&format!(
                        "rejected extension: {} [{}]",
                        refusal.reason, refusal.code
                    ));
                    let mut out = results.get().init_outcome().init_unauthorized();
                    out.set_reason(&refusal.reason);
                    out.set_retryable(refusal.retryable);
                    out.set_code(refusal.code);
                    gate.close_soon();
                    return Ok(());
                }
            };
            let known = tools::bundled(&hello.manifest_hash);
            let manifest_in_sync = known.is_some();
            let welcome_hash = if manifest_in_sync {
                hello.manifest_hash.clone()
            } else {
                tools::chromium().hash.clone()
            };
            let proof = handshake::server_proof(
                &secret,
                &transcript,
                crate::VERSION,
                &welcome_hash,
                manifest_in_sync,
                sealed.as_deref(),
            )
            .map_err(|error| capnp::Error::failed(format!("handshake failed: {error}")))?;
            let serial = gate.daemon.next_serial();
            let link = Rc::new(Link::new(
                serial,
                hello,
                page,
                ui,
                Rc::clone(&gate.conn),
                gate.daemon.touch(),
            ));
            *gate.link.borrow_mut() = Some(Rc::clone(&link));
            *gate.state.borrow_mut() = State::Done;
            let session: extension_session::Client = capnp_rpc::new_client(
                crate::session::Session::new(gate.daemon.clone(), Rc::downgrade(&link)),
            );
            {
                let mut welcome = results.get().init_outcome().init_welcome();
                welcome.set_daemon_version(crate::VERSION);
                welcome.set_protocol_version(SOCKET_PROTOCOL_VERSION);
                welcome.set_manifest_hash(&welcome_hash);
                welcome.set_manifest_in_sync(manifest_in_sync);
                welcome.set_proof(&proof);
                welcome.set_sealed_session_key(sealed.as_deref().unwrap_or(""));
                welcome.set_session(session);
            }
            // Listed after the welcome is built, so no push can reach the browser ahead of it.
            let daemon = gate.daemon.clone();
            tokio::task::spawn_local(async move {
                tokio::task::yield_now().await;
                daemon.accept_link(link);
            });
            Ok(())
        }
    }
}

struct Refusal {
    reason: String,
    retryable: bool,
    code: &'static str,
}

/// PBKDF2 is ~100 ms of CPU: off the event loop.
async fn secret_for(code: String, transcript: &Transcript) -> String {
    let transcript = transcript.clone();
    tokio::task::spawn_blocking(move || handshake::pairing_secret(&code, &transcript))
        .await
        .unwrap_or_default()
}

/// The grant held for this browser first, then each live pairing code.
async fn prove_pairing(
    daemon: &Daemon,
    proof: &str,
    transcript: &Transcript,
    install: &auth_store::Install,
) -> Result<(String, Option<String>), Refusal> {
    let granted = daemon
        .0
        .desk
        .borrow_mut()
        .grant_for(&install.install_id, &install.origin);
    let mut candidates: Vec<(String, String)> = granted
        .map(|grant| (grant.code, grant.paired_by))
        .into_iter()
        .collect();
    candidates.extend(
        auth_store::pending_pairings()
            .into_iter()
            .map(|code| (code, "code".to_owned())),
    );
    for (code, paired_by) in candidates {
        let secret = secret_for(code.clone(), transcript).await;
        let expected = handshake::client_proof(&secret, transcript).unwrap_or_default();
        if expected.is_empty() || !handshake::same_proof(proof, &expected) {
            continue;
        }
        if paired_by == "code" {
            auth_store::consume_pairing(&code);
            auth_store::approve_install(
                &install.install_id,
                &install.origin,
                install.browser.as_deref(),
                None,
            );
        }
        daemon.0.desk.borrow_mut().consume(&install.install_id);
        let session = auth_store::create_session(install, &paired_by);
        let sealed = handshake::seal_session_key(&secret, transcript, &session.key).ok();
        log::log(&format!(
            "paired {} (extension {}) [{paired_by}]",
            install.browser.as_deref().unwrap_or(&install.origin),
            install.extension_version
        ));
        crate::reregister_native_host();
        return Ok((secret, sealed));
    }
    Err(Refusal {
        reason: "That pairing code is wrong or expired. Run \"vexdom pair\" for a new one.".into(),
        retryable: true,
        code: "bad-code",
    })
}

async fn prove_session(
    proof: &str,
    transcript: &Transcript,
    install: &auth_store::Install,
) -> Result<(String, Option<String>), Refusal> {
    for session in auth_store::session_candidates(&install.install_id, &install.origin) {
        let Ok(expected) = handshake::client_proof(&session.key, transcript) else {
            continue;
        };
        if handshake::same_proof(proof, &expected) {
            auth_store::claim_session(&session.key, install);
            return Ok((session.key, None));
        }
    }
    let code = if auth_store::block_of(&install.install_id).is_some() {
        "revoked"
    } else {
        "unknown"
    };
    Err(Refusal {
        reason: "This browser is no longer paired. Run \"vexdom pair\" to pair again.".into(),
        retryable: false,
        code,
    })
}
