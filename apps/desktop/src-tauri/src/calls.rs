//! Live call signalling over the same encrypted peer session as messages.
//!
//! The shell only relays signalling frames and records the call log; the SDP and ICE
//! bodies are opaque to it. Media itself never leaves the two browsers' WebRTC stack.

use alver_core::{
    payloads::{
        kind, CallAnswerPayload, CallBusyPayload, CallEndPayload, CallOfferPayload,
        CallRejectPayload, IceCandidatePayload,
    },
    store_ext::{CallEvent, CallEventKind},
    Envelope, Hlc, PROTOCOL_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use crate::{commands, start_outbox_retry, OverlayState};

/// One signalling frame as the webview sends it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallSignal {
    pub call_id: String,
    /// `offer`, `answer`, `ice`, `end`, `reject` or `busy`.
    pub kind: String,
    #[serde(default)]
    pub media: String,
    #[serde(default)]
    pub sdp: String,
    #[serde(default)]
    pub candidate: String,
    #[serde(default)]
    pub sdp_mid: Option<String>,
    #[serde(default)]
    pub sdp_index: Option<u32>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub accepted: bool,
}

/// A signalling frame as the webview receives it, with the peer it came from.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomingCallSignal {
    pub chat_id: String,
    pub caller: String,
    pub signal: CallSignal,
}

pub(crate) fn handled_kind(kind: &str) -> bool {
    matches!(
        kind,
        kind::CALL_OFFER
            | kind::CALL_ANSWER
            | kind::CALL_END
            | kind::CALL_REJECT
            | kind::CALL_BUSY
            | kind::ICE_CANDIDATE
    )
}

fn my_id(app: &AppHandle) -> String {
    app.state::<OverlayState>().identity.peer_id()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn parse_call(call_id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(call_id).map_err(|_| "Call id is not a UUID.".to_owned())
}

/// Converts one webview signal into the protocol frame it names.
fn frame(signal: &CallSignal, chat_id: &str, from: &str) -> Result<Envelope<Value>, String> {
    let call_id = parse_call(&signal.call_id)?;
    let stamp = now_ms();
    let (kind, payload) = match signal.kind.as_str() {
        "offer" => (
            kind::CALL_OFFER,
            serde_json::to_value(CallOfferPayload {
                call_id,
                chat_id: chat_id.to_owned(),
                media: if signal.media.is_empty() {
                    "audio".into()
                } else {
                    signal.media.clone()
                },
                sdp: signal.sdp.clone(),
                group: false,
            }),
        ),
        "answer" => (
            kind::CALL_ANSWER,
            serde_json::to_value(CallAnswerPayload {
                call_id,
                accepted: signal.accepted,
                sdp: signal.sdp.clone(),
            }),
        ),
        "ice" => (
            kind::ICE_CANDIDATE,
            serde_json::to_value(IceCandidatePayload {
                call_id,
                candidate: signal.candidate.clone(),
                sdp_mid: signal.sdp_mid.clone(),
                sdp_index: signal.sdp_index,
            }),
        ),
        "end" => (
            kind::CALL_END,
            serde_json::to_value(CallEndPayload {
                call_id,
                reason: signal.reason.clone(),
                ended_ms: stamp,
            }),
        ),
        "reject" => (
            kind::CALL_REJECT,
            serde_json::to_value(CallRejectPayload {
                call_id,
                reason: signal.reason.clone(),
            }),
        ),
        "busy" => (
            kind::CALL_BUSY,
            serde_json::to_value(CallBusyPayload { call_id }),
        ),
        other => return Err(format!("Unknown call signal '{other}'.")),
    };
    Ok(Envelope {
        v: PROTOCOL_VERSION,
        id: Uuid::now_v7(),
        kind: kind.to_owned(),
        from: from.to_owned(),
        to: chat_id.to_owned(),
        hlc: Hlc {
            physical_ms: stamp,
            logical: 0,
        },
        payload: payload.map_err(|error| error.to_string())?,
    })
}

/// The call-log row a signal appends, if the kind is one the history keeps.
fn event_for(
    signal: &CallSignal,
    chat_id: &str,
    peer_id: &str,
    accepted: Option<bool>,
) -> Option<CallEvent> {
    let kind = match signal.kind.as_str() {
        "offer" => CallEventKind::Offer,
        "answer" if accepted == Some(false) => CallEventKind::Reject,
        "answer" => CallEventKind::Answer,
        "end" => CallEventKind::End,
        "reject" => CallEventKind::Reject,
        "busy" => CallEventKind::Busy,
        _ => return None,
    };
    let stamp = now_ms();
    Some(CallEvent {
        id: Uuid::now_v7(),
        chat_id: chat_id.to_owned(),
        call_id: parse_call(&signal.call_id).ok()?,
        peer_id: peer_id.to_owned(),
        kind,
        accepted,
        started_ms: stamp,
        ended_ms: matches!(
            kind,
            CallEventKind::End | CallEventKind::Reject | CallEventKind::Busy
        )
        .then_some(stamp),
        note: signal.reason.clone().unwrap_or_default(),
    })
}

fn record(app: &AppHandle, event: Option<CallEvent>) {
    let Some(event) = event else {
        return;
    };
    let state = app.state::<OverlayState>();
    let Ok(mut store) = state.store.lock() else {
        return;
    };
    if let Err(error) = store.save_call_event(&event) {
        eprintln!("Could not record a call event: {error}");
    }
}

/// Sends one signalling frame from the webview to a peer and records it locally.
#[tauri::command]
pub(crate) fn send_call_signal(
    app: AppHandle,
    chat_id: String,
    signal: CallSignal,
) -> Result<(), String> {
    let chat_id = chat_id.trim().to_owned();
    if chat_id.is_empty() {
        return Err("A chat id is required.".into());
    }
    let from = my_id(&app);
    let envelope = frame(&signal, &chat_id, &from)?;
    record(&app, event_for(&signal, &chat_id, &from, Some(true)));
    let state = app.state::<OverlayState>();
    let mut store = state
        .store
        .lock()
        .map_err(|_| "message store lock poisoned")?;
    store.enqueue(&chat_id, &envelope)?;
    drop(store);
    let beacon = state
        .endpoints
        .lock()
        .map_err(|_| "peer endpoint lock poisoned".to_owned())?
        .get(&chat_id)
        .cloned();
    match beacon {
        Some(beacon) => commands::flush_outbox(&app, &chat_id, &beacon),
        // Signalling is often the first frame after a reconnect, so a missing endpoint
        // must keep the frame queued instead of dropping it.
        None => start_outbox_retry(app.clone(), chat_id),
    }
    let _ = app.emit("calls-changed", ());
    Ok(())
}

/// Receives a call frame from a peer, appends it to the call log and hands it to the UI.
pub(crate) fn handle_signal(
    app: &AppHandle,
    remote: &str,
    envelope: &Envelope<Value>,
) -> Result<(), String> {
    let signal = match envelope.kind.as_str() {
        kind::CALL_OFFER => CallSignal {
            call_id: field::<Uuid>(&envelope.payload, "call_id")?.to_string(),
            kind: "offer".into(),
            media: envelope
                .payload
                .get("media")
                .and_then(Value::as_str)
                .unwrap_or("audio")
                .to_owned(),
            sdp: envelope
                .payload
                .get("sdp")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            candidate: String::new(),
            sdp_mid: None,
            sdp_index: None,
            reason: None,
            accepted: false,
        },
        kind::CALL_ANSWER => CallSignal {
            call_id: field::<Uuid>(&envelope.payload, "call_id")?.to_string(),
            kind: "answer".into(),
            media: String::new(),
            sdp: envelope
                .payload
                .get("sdp")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            candidate: String::new(),
            sdp_mid: None,
            sdp_index: None,
            reason: None,
            accepted: envelope
                .payload
                .get("accepted")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        kind::ICE_CANDIDATE => CallSignal {
            call_id: field::<Uuid>(&envelope.payload, "call_id")?.to_string(),
            kind: "ice".into(),
            media: String::new(),
            sdp: String::new(),
            candidate: envelope
                .payload
                .get("candidate")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            sdp_mid: envelope
                .payload
                .get("sdp_mid")
                .and_then(Value::as_str)
                .map(str::to_owned),
            sdp_index: envelope
                .payload
                .get("sdp_index")
                .and_then(Value::as_u64)
                .map(|value| value as u32),
            reason: None,
            accepted: false,
        },
        kind::CALL_END => CallSignal {
            call_id: field::<Uuid>(&envelope.payload, "call_id")?.to_string(),
            kind: "end".into(),
            media: String::new(),
            sdp: String::new(),
            candidate: String::new(),
            sdp_mid: None,
            sdp_index: None,
            reason: envelope
                .payload
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_owned),
            accepted: false,
        },
        kind::CALL_REJECT => CallSignal {
            call_id: field::<Uuid>(&envelope.payload, "call_id")?.to_string(),
            kind: "reject".into(),
            media: String::new(),
            sdp: String::new(),
            candidate: String::new(),
            sdp_mid: None,
            sdp_index: None,
            reason: envelope
                .payload
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_owned),
            accepted: false,
        },
        kind::CALL_BUSY => CallSignal {
            call_id: field::<Uuid>(&envelope.payload, "call_id")?.to_string(),
            kind: "busy".into(),
            media: String::new(),
            sdp: String::new(),
            candidate: String::new(),
            sdp_mid: None,
            sdp_index: None,
            reason: None,
            accepted: false,
        },
        other => return Err(format!("unknown call frame {other}")),
    };
    let accepted = (signal.kind == "answer").then_some(signal.accepted);
    record(
        app,
        event_for(&signal, remote, envelope.from.as_str(), accepted),
    );
    let _ = app.emit(
        "call-signal",
        IncomingCallSignal {
            chat_id: remote.to_owned(),
            caller: envelope.from.clone(),
            signal,
        },
    );
    let _ = app.emit("calls-changed", ());
    Ok(())
}

fn field<T: serde::de::DeserializeOwned>(payload: &Value, name: &str) -> Result<T, String> {
    serde_json::from_value(
        payload
            .get(name)
            .cloned()
            .ok_or_else(|| format!("call frame is missing {name}"))?,
    )
    .map_err(|error| format!("call frame has an invalid {name}: {error}"))
}
