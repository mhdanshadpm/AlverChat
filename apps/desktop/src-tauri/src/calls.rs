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
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use crate::{commands, publish, start_outbox_retry, OverlayState};

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

/// A call the shell believes is still up, derived only from the signalling frames it
/// relayed. The floating icon reads this to draw its In-Call state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LiveCall {
    pub call_id: Uuid,
    pub chat_id: String,
    /// False while the offer is still waiting for an answer.
    pub ongoing: bool,
    /// Wall-clock ms of the answer that connected it; 0 while ringing.
    pub started_ms: u64,
    /// Wall-clock ms the registry first saw this call, for a stable pick.
    pub seen_ms: u64,
}

/// Folds one recorded call-log event into the live registry.
/// Returns whether the registry changed, so callers repaint only on a real transition.
pub(crate) fn apply_call_event(
    calls: &mut HashMap<Uuid, LiveCall>,
    event: &CallEvent,
    at_ms: u64,
) -> bool {
    match event.kind {
        CallEventKind::Offer => {
            if calls.contains_key(&event.call_id) {
                return false;
            }
            calls.insert(
                event.call_id,
                LiveCall {
                    call_id: event.call_id,
                    chat_id: event.chat_id.clone(),
                    ongoing: false,
                    started_ms: 0,
                    seen_ms: at_ms,
                },
            );
            true
        }
        CallEventKind::Answer => {
            // `event_for` only records an Answer once it is accepted, so this is the
            // frame that turns a ringing call into a live one.
            let Some(call) = calls.get_mut(&event.call_id) else {
                return false;
            };
            if call.ongoing {
                return false;
            }
            call.ongoing = true;
            call.started_ms = at_ms;
            true
        }
        CallEventKind::End | CallEventKind::Reject | CallEventKind::Busy => {
            calls.remove(&event.call_id).is_some()
        }
    }
}

/// The call the icon should show: the oldest connected one, or the oldest ringing one
/// while nothing has been answered.
pub(crate) fn pick_live_call(calls: &HashMap<Uuid, LiveCall>) -> Option<&LiveCall> {
    let mut live: Vec<&LiveCall> = calls.values().collect();
    if live.is_empty() {
        return None;
    }
    live.sort_by(|a, b| {
        b.ongoing
            .cmp(&a.ongoing)
            .then(a.started_ms.cmp(&b.started_ms))
            .then(a.seen_ms.cmp(&b.seen_ms))
            .then(a.call_id.cmp(&b.call_id))
    });
    live.first().copied()
}

/// Ends every call belonging to one chat. A peer that vanishes from the LAN never sends
/// a hang-up, so presence loss is the only signal that its call is over.
pub(crate) fn drop_calls_for_chat(calls: &mut HashMap<Uuid, LiveCall>, chat_id: &str) -> bool {
    let before = calls.len();
    calls.retain(|_, call| call.chat_id != chat_id);
    calls.len() != before
}

fn record(app: &AppHandle, event: Option<CallEvent>) {
    let Some(event) = event else {
        return;
    };
    let state = app.state::<OverlayState>();
    {
        let Ok(mut store) = state.store.lock() else {
            return;
        };
        if let Err(error) = store.save_call_event(&event) {
            eprintln!("Could not record a call event: {error}");
        }
    }
    let changed = state
        .live_calls
        .lock()
        .map(|mut calls| apply_call_event(&mut calls, &event, now_ms()))
        .unwrap_or(false);
    if changed {
        publish(app);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn event(call: u128, chat: &str, kind: CallEventKind, accepted: Option<bool>) -> CallEvent {
        CallEvent {
            id: Uuid::from_u128(call * 1000),
            chat_id: chat.to_owned(),
            call_id: Uuid::from_u128(call),
            peer_id: chat.to_owned(),
            kind,
            accepted,
            started_ms: 1,
            ended_ms: None,
            note: String::new(),
        }
    }

    #[test]
    fn one_call_moves_from_ringing_to_live_to_over() {
        let mut calls = HashMap::new();
        assert!(apply_call_event(
            &mut calls,
            &event(1, "peer-1", CallEventKind::Offer, None),
            10
        ));
        assert_eq!(calls.len(), 1);
        assert!(!calls[&Uuid::from_u128(1)].ongoing);
        assert_eq!(calls[&Uuid::from_u128(1)].started_ms, 0);
        // A re-offer for the same call must not restart the clock or repaint.
        assert!(!apply_call_event(
            &mut calls,
            &event(1, "peer-1", CallEventKind::Offer, None),
            20
        ));
        assert!(apply_call_event(
            &mut calls,
            &event(1, "peer-1", CallEventKind::Answer, Some(true)),
            30
        ));
        assert!(calls[&Uuid::from_u128(1)].ongoing);
        assert_eq!(calls[&Uuid::from_u128(1)].started_ms, 30);
        // ICE and other frames never reach here, but a second answer must stay quiet.
        assert!(!apply_call_event(
            &mut calls,
            &event(1, "peer-1", CallEventKind::Answer, Some(true)),
            40
        ));
        assert!(apply_call_event(
            &mut calls,
            &event(1, "peer-1", CallEventKind::End, None),
            50
        ));
        assert!(calls.is_empty());
        assert!(!apply_call_event(
            &mut calls,
            &event(1, "peer-1", CallEventKind::End, None),
            60
        ));
    }

    #[test]
    fn a_refused_call_ends_the_icon_state() {
        for kind in [CallEventKind::Reject, CallEventKind::Busy] {
            let mut calls = HashMap::new();
            apply_call_event(
                &mut calls,
                &event(7, "peer-7", CallEventKind::Offer, None),
                10,
            );
            assert!(
                apply_call_event(&mut calls, &event(7, "peer-7", kind, None), 20),
                "{kind:?}"
            );
            assert!(calls.is_empty());
        }
    }

    #[test]
    fn losing_a_peer_ends_only_its_call() {
        let mut calls = HashMap::new();
        apply_call_event(
            &mut calls,
            &event(2, "peer-a", CallEventKind::Offer, None),
            10,
        );
        apply_call_event(
            &mut calls,
            &event(3, "peer-b", CallEventKind::Offer, None),
            12,
        );
        assert!(drop_calls_for_chat(&mut calls, "peer-a"));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[&Uuid::from_u128(3)].chat_id, "peer-b");
        assert!(!drop_calls_for_chat(&mut calls, "peer-a"));
        // An answer for a chat the registry never saw must not invent a call.
        assert!(!apply_call_event(
            &mut calls,
            &event(9, "peer-c", CallEventKind::Answer, Some(true)),
            20
        ));
        assert!(calls.values().all(|call| call.chat_id != "peer-c"));
    }

    #[test]
    fn the_pick_prefers_the_oldest_live_call() {
        let mut calls = HashMap::new();
        assert!(pick_live_call(&calls).is_none());
        apply_call_event(
            &mut calls,
            &event(4, "peer-a", CallEventKind::Offer, None),
            10,
        );
        apply_call_event(
            &mut calls,
            &event(5, "peer-b", CallEventKind::Offer, None),
            11,
        );
        assert_eq!(
            pick_live_call(&calls).map(|call| call.call_id),
            Some(Uuid::from_u128(4)),
            "the first ringing call wins"
        );
        apply_call_event(
            &mut calls,
            &event(5, "peer-b", CallEventKind::End, None),
            12,
        );
        apply_call_event(
            &mut calls,
            &event(6, "peer-c", CallEventKind::Offer, None),
            13,
        );
        apply_call_event(
            &mut calls,
            &event(6, "peer-c", CallEventKind::Answer, Some(true)),
            14,
        );
        assert_eq!(
            pick_live_call(&calls).map(|call| call.call_id),
            Some(Uuid::from_u128(6)),
            "a connected call outranks a ringing one"
        );
        assert_eq!(pick_live_call(&calls).map(|call| call.started_ms), Some(14));
    }
}
