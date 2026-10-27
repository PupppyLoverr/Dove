//! The next step after an action (Hermes `_classify_action_result`):
//! `done`, `verify_fresh_state` or `escalate`. Escalation is advisory: it
//! never overrides a confirmed effect nor licenses repeating input.

use serde_json::{Value, json};

use crate::types::ActionResult;

pub fn classify(res: &ActionResult) -> Value {
    if res.effect.as_deref() == Some("confirmed") || res.verified == Some(true) {
        return json!({"decision": "done"});
    }
    if res.effect.as_deref() == Some("unverifiable") {
        return json!({"decision": "verify_fresh_state", "hint": "Input was delivered but not confirmed. Re-capture \
            and check the result BEFORE any retry; do not repeat the input on an escalation recommendation alone."});
    }
    if res.effect.as_deref() == Some("suspected_noop") || !res.ok || res.code.is_some() {
        if res.code.as_deref() == Some("stale") {
            return json!({"decision": "recapture", "hint": "The element index came from an older capture. \
                Capture again and use the new index."});
        }
        let int = |k: &str| res.meta.get(k).and_then(Value::as_i64);
        // Zero characters landed on the driver's own partial-delivery
        // verdict: the field drops synthetic keystrokes, so no delivery rung
        // helps. set_value writes through the accessibility API instead.
        if matches!(res.action.as_str(), "type" | "type_text")
            && res.code.as_deref() == Some("type_text_incomplete")
            && int("requested_chars").is_some_and(|r| r > 0)
            && int("delivered_chars").is_some_and(|d| d <= 0)
        {
            return json!({"decision": "escalate", "recommended": "set_value", "hint": "0 characters landed: this \
                input drops synthetic keystrokes, so no delivery rung will fix it. Use action='set_value' on the \
                field's element index instead (it writes through the accessibility API). Capture first if the \
                index is stale."});
        }
        let mut out = json!({"decision": "escalate", "hint": "The input likely did not land. Climb one rung \
            following `recommended`: 'px' means re-issue by coordinate; 'foreground' (or a failed pixel click) \
            means re-issue with delivery_mode='foreground' (separate approval). Do not predict the rung from the \
            app being Electron or Chromium; react to this signal."});
        if let Some(r) = res.escalation.as_ref().and_then(|e| e.get("recommended")) {
            out["recommended"] = r.clone();
        }
        return out;
    }
    json!({"decision": "verify_fresh_state", "hint": "Transport succeeded but the effect is unproven. Re-capture \
        and confirm before continuing."})
}

/// The action's JSON payload: transport `ok`, the driver's verdict fields
/// when present, and the `verdict` the model follows.
pub fn payload(res: &ActionResult) -> Value {
    let mut out = json!({"ok": res.ok, "action": res.action});
    if !res.message.is_empty() {
        out["message"] = json!(res.message);
    }
    if let Some(v) = res.verified {
        out["verified"] = json!(v);
    }
    for (k, v) in [
        ("effect", &res.effect),
        ("path", &res.path),
        ("delivery_mode", &res.delivery_mode),
        ("code", &res.code),
    ] {
        if let Some(v) = v {
            out[k] = json!(v);
        }
    }
    if let Some(e) = &res.escalation {
        out["escalation"] = Value::Object(e.clone());
    }
    if let Some(d) = res.degraded {
        out["degraded"] = json!(d);
    }
    out["verdict"] = classify(res);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(effect: Option<&str>, ok: bool, code: Option<&str>) -> ActionResult {
        ActionResult {
            ok,
            action: "click".into(),
            effect: effect.map(Into::into),
            code: code.map(Into::into),
            ..ActionResult::default()
        }
    }

    #[test]
    fn the_ladder() {
        assert_eq!(classify(&r(Some("confirmed"), true, None))["decision"], "done");
        assert_eq!(classify(&r(Some("unverifiable"), true, None))["decision"], "verify_fresh_state");
        assert_eq!(classify(&r(None, true, None))["decision"], "verify_fresh_state");
        let mut noop = r(Some("suspected_noop"), true, None);
        noop.escalation = Some(serde_json::from_value(json!({"recommended": "foreground"})).unwrap());
        let v = classify(&noop);
        assert_eq!((v["decision"].as_str(), v["recommended"].as_str()), (Some("escalate"), Some("foreground")));
        assert_eq!(classify(&r(None, false, Some("stale")))["decision"], "recapture");
        // A confirmed effect wins over an escalation recommendation.
        let mut done = r(Some("confirmed"), true, Some("background_unavailable"));
        done.escalation = noop.escalation.clone();
        assert_eq!(classify(&done)["decision"], "done");
    }

    #[test]
    fn zero_chars_climbs_to_set_value() {
        let mut t = r(None, false, Some("type_text_incomplete"));
        t.action = "type".into();
        t.meta = serde_json::from_value(json!({"requested_chars": 5, "delivered_chars": 0})).unwrap();
        assert_eq!(classify(&t)["recommended"], "set_value");
        // The synthesis budget code keeps the ordinary escalation.
        t.code = Some("type_text_synthesis_budget_exceeded".into());
        assert!(classify(&t).get("recommended").is_none());
    }
}
