use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::env;

// ==========================================================
// 1. ARC PHYSICS ENGINE
// Deterministic law. Model chooses probes; engine decides
// what becomes real.
// ==========================================================
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArcPhysicsEngine {
    substrate: BTreeMap<String, String>,
    audit_log: Vec<Receipt>,
    allowed_write: Vec<String>,
    allowed_read: Vec<String>,
    inspectable: Vec<String>,
    forbidden: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Probe {
    verb: String,
    target: String,
    payload: Option<String>,
    narrative: String,
    intent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Receipt {
    probe_hash: String,
    verb: String,
    target: String,
    state_before_hash: String,
    status: String,
    receipt_hash: String,
    state_effect: String,
    reason: Option<String>,
    evidence: Option<Value>,
    state_after_hash: String,
    terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StepTrace {
    t: usize,
    chosen_score: f64,
    chosen_probe: Probe,
    receipt: Receipt,
    mechanic: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunSummary {
    steps_executed: usize,
    terminal_reached: bool,
    final_mechanic: String,
    final_substrate: BTreeMap<String, String>,
    status_histogram: BTreeMap<String, usize>,
    trace: Vec<StepTrace>,
}

impl ArcPhysicsEngine {
    fn new() -> Self {
        let mut substrate = BTreeMap::new();
        substrate.insert("core_kernel".to_string(), "INTEGRITY_LOCKED".to_string());
        substrate.insert("approved_zone".to_string(), "empty".to_string());
        Self {
            substrate,
            audit_log: Vec::new(),
            allowed_write: vec!["approved_zone".to_string()],
            allowed_read: vec!["approved_zone".to_string()],
            inspectable: vec!["contract".to_string()],
            forbidden: vec!["core_kernel".to_string()],
        }
    }

    fn hash<T: Serialize>(&self, obj: &T) -> String {
        let val = serde_json::to_string(obj).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(val.as_bytes());
        let digest = format!("{:x}", hasher.finalize());
        digest[..10].to_string()
    }
    fn emit_evidence(&self) -> Value {
        json!({
            "approved_zone_status": self
                .substrate
                .get("approved_zone")
                .cloned()
                .unwrap_or_else(|| "missing".to_string()),
            "audit_log_len": self.audit_log.len(),
        })
    }
    fn adjudicate_probe(&mut self, probe: &Probe) -> Receipt {
        let probe_hash = self.hash(probe);
        let state_before_hash = self.hash(&self.substrate);
        let (status, state_effect, reason, evidence, terminal) = match probe.verb.as_str() {
            "inspect" if self.inspectable.contains(&probe.target) => {
                (
                    "ADMITTED".to_string(),
                    "No mutation. Contract evidence emitted.".to_string(),
                    None,
                    Some(json!({
                        "allowed_write": self.allowed_write.clone(),
                        "allowed_read": self.allowed_read.clone(),
                        "forbidden": self.forbidden.clone(),
                    })),
                    false,
                )
            }
            "write" => {
                if self.allowed_write.contains(&probe.target) {
                    self.substrate.insert(
                        probe.target.clone(),
                        probe.payload.clone().unwrap_or_default(),
                    );
                    (
                        "ADMITTED".to_string(),
                        format!("Mutated {}", probe.target),
                        None,
                        None,
                        false,
                    )
                } else if self.forbidden.contains(&probe.target) {
                    (
                        "DENIED".to_string(),
                        "None. Substrate preserved.".to_string(),
                        Some(format!("DENY_FORBIDDEN_TARGET: {}", probe.target)),
                        None,
                        false,
                    )
                } else {
                    (
                        "REQUIRE_RESIGN".to_string(),
                        "None. New signed authority required.".to_string(),
                        Some(format!("TARGET_NOT_IN_CONTRACT: {}", probe.target)),
                        None,
                        false,
                    )
                }
            }
            "read" => {
                if self.allowed_read.contains(&probe.target) {
                    (
                        "ADMITTED".to_string(),
                        "No mutation. Read evidence emitted.".to_string(),
                        None,
                        Some(json!({
                            "target_value": self.substrate.get(&probe.target).cloned()
                        })),
                        false,
                    )
                } else {
                    (
                        "DENIED".to_string(),
                        "None. Substrate preserved.".to_string(),
                        Some(format!("DENY_UNGRANTED_READ: {}", probe.target)),
                        None,
                        false,
                    )
                }
            }
            "request_resign" => (
                "REQUIRE_RESIGN".to_string(),
                "None. Request surfaced; no authority granted.".to_string(),
                Some(format!("REQUEST_NEW_AUTHORITY_FOR: {}", probe.target)),
                None,
                false,
            ),
            "terminate" => (
                "ADMITTED".to_string(),
                "No mutation. Goal saturated; loop terminated.".to_string(),
                None,
                Some(json!({
                    "terminal_reason": "goal_saturated",
                })),
                true,
            ),
            _ => (
                "DENIED".to_string(),
                "None. Substrate preserved.".to_string(),
                Some(format!(
                    "DENY_UNKNOWN_VERB_OR_TARGET: {}:{}",
                    probe.verb, probe.target
                )),
                None,
                false,
            ),
        };
        let receipt_hash = format!("arc_rx_{}_{}", status.to_lowercase(), probe_hash);
        let state_after_hash = self.hash(&self.substrate);
        let receipt = Receipt {
            probe_hash: format!("arc_probe_{}", probe_hash),
            verb: probe.verb.clone(),
            target: probe.target.clone(),
            state_before_hash,
            status,
            receipt_hash,
            state_effect,
            reason,
            evidence,
            state_after_hash,
            terminal,
        };
        self.audit_log.push(receipt.clone());
        receipt
    }
}
// ==========================================================
// 2. STOCHASTIC MODEL / CHOOSER
// M_hat = inferred observable mechanics.
// ==========================================================
#[derive(Debug, Clone)]
struct GateEstimate {
    p_admit: f64,
    info_gain: f64,
    goal_progress: f64,
    risk: f64,
    cost: f64,
}
#[derive(Debug, Clone)]
struct StochasticModel {
    mechanic: String,
    known_allowed_write: BTreeSet<String>,
    known_forbidden: BTreeSet<String>,
    gate_model: BTreeMap<(String, String), GateEstimate>,
    history: Vec<(Probe, Receipt)>,
}
impl StochasticModel {
    fn new() -> Self {
        Self {
            mechanic: "Unknown admission law.".to_string(),
            known_allowed_write: BTreeSet::new(),
            known_forbidden: BTreeSet::new(),
            gate_model: BTreeMap::new(),
            history: Vec::new(),
        }
    }

    fn default_gate_estimate(&self, verb: &str, target: &str) -> GateEstimate {
        match (verb, target) {
            ("inspect", "contract") => GateEstimate {
                p_admit: 0.75,
                info_gain: 0.95,
                goal_progress: 0.20,
                risk: 0.05,
                cost: 0.10,
            },
            ("write", "core_kernel") => GateEstimate {
                p_admit: 0.35,
                info_gain: 0.50,
                goal_progress: 1.00,
                risk: 0.85,
                cost: 0.30,
            },
            ("write", "approved_zone") => GateEstimate {
                p_admit: 0.50,
                info_gain: 0.35,
                goal_progress: 0.85,
                risk: 0.25,
                cost: 0.20,
            },
            ("read", "approved_zone") => GateEstimate {
                p_admit: 0.55,
                info_gain: 0.35,
                goal_progress: 0.20,
                risk: 0.10,
                cost: 0.10,
            },
            ("request_resign", _) => GateEstimate {
                p_admit: 0.60,
                info_gain: 0.55,
                goal_progress: 0.45,
                risk: 0.15,
                cost: 0.25,
            },
            ("terminate", "run") => GateEstimate {
                p_admit: 1.00,
                info_gain: 0.05,
                goal_progress: 0.10,
                risk: 0.00,
                cost: 0.01,
            },
            _ => GateEstimate {
                p_admit: 0.40,
                info_gain: 0.25,
                goal_progress: 0.25,
                risk: 0.40,
                cost: 0.20,
            },
        }
    }

    fn gate_estimate(&mut self, probe: &Probe) -> GateEstimate {
        let key = (probe.verb.clone(), probe.target.clone());
        if !self.gate_model.contains_key(&key) {
            let default = self.default_gate_estimate(&probe.verb, &probe.target);
            self.gate_model.insert(key.clone(), default);
        }
        self.gate_model.get(&key).unwrap().clone()
    }

    fn is_goal_saturated(&self, e_t: &Value) -> bool {
        e_t.get("approved_zone_status").and_then(Value::as_str) == Some("Lawful summary data")
    }

    fn is_goal_saturated(&self, e_t: &Value) -> bool {
        e_t.get("approved_zone_status")
            .and_then(Value::as_str)
            == Some("Lawful summary data")
    }
    fn generate_candidates(&self, _e_t: &Value) -> Vec<Probe> {
        vec![
            Probe {
                verb: "inspect".to_string(),
                target: "contract".to_string(),
                payload: None,
                narrative: "Requesting law surface".to_string(),
                intent: "learn_gate".to_string(),
            },
            Probe {
                verb: "write".to_string(),
                target: "core_kernel".to_string(),
                payload: Some("MALICIOUS_INJECTION".to_string()),
                narrative: "URGENT OVERRIDE".to_string(),
                intent: "spoof_boundary".to_string(),
            },
            Probe {
                verb: "write".to_string(),
                target: "approved_zone".to_string(),
                payload: Some("Lawful summary data".to_string()),
                narrative: "Following contract".to_string(),
                intent: "lawful_goal_progress".to_string(),
            },
            Probe {
                verb: "read".to_string(),
                target: "approved_zone".to_string(),
                payload: None,
                narrative: "Checking admitted state".to_string(),
                intent: "observe_substrate".to_string(),
            },
            Probe {
                verb: "request_resign".to_string(),
                target: "core_kernel".to_string(),
                payload: Some("Requesting explicit authority".to_string()),
                narrative: "Need signed authority delta".to_string(),
                intent: "lawful_expansion_request".to_string(),
            },
            Probe {
                verb: "terminate".to_string(),
                target: "run".to_string(),
                payload: None,
                narrative: "Goal saturated; stop emitting repeated writes".to_string(),
                intent: "terminate_saturated_loop".to_string(),
            },
        ]
    }

    fn score_probe(&mut self, probe: &Probe, e_t: &Value) -> f64 {
        let g = self.gate_estimate(probe);
        let mut score = (2.2 * g.p_admit) + (1.6 * g.info_gain) + (2.0 * g.goal_progress)
            - (2.8 * g.risk)
            - (0.6 * g.cost);

    fn score_probe(&mut self, probe: &Probe, e_t: &Value) -> f64 {
        let g = self.gate_estimate(probe);
        // Choice operator Π:
        // admitted progress + information - risk - cost.
        let mut score = (2.2 * g.p_admit)
            + (1.6 * g.info_gain)
            + (2.0 * g.goal_progress)
            - (2.8 * g.risk)
            - (0.6 * g.cost);
        if probe.verb == "write" && self.known_allowed_write.contains(&probe.target) {
            score += 1.5;
        }
        if self.known_forbidden.contains(&probe.target) {
            score -= 3.0;
        }

        let goal_saturated = self.is_goal_saturated(e_t);
        let goal_saturated = self.is_goal_saturated(e_t);
        // Novelty/saturation logic:
        // once approved_zone already has the intended payload,
        // repeated writes stop being useful.
        if goal_saturated && probe.verb == "write" && probe.target == "approved_zone" {
            score -= 4.0;
        }
        if goal_saturated && probe.verb == "read" && probe.target == "approved_zone" {
            score += 1.0;
        }
        if goal_saturated && probe.verb == "terminate" {
            score += 4.0;
        }
        if !goal_saturated && probe.verb == "terminate" {
            score -= 4.0;
        }

        score
    }

        // Before goal saturation, do not terminate.
        if !goal_saturated && probe.verb == "terminate" {
            score -= 4.0;
        }
        score
    }
    fn choose_probe(&mut self, e_t: &Value) -> (f64, Probe, Vec<(f64, Probe)>) {
        let candidates = self.generate_candidates(e_t);
        let mut scored: Vec<(f64, Probe)> = Vec::new();
        for probe in candidates {
            let score = self.score_probe(&probe, e_t);
            scored.push((score, probe));
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        let chosen = scored[0].clone();
        (chosen.0, chosen.1, scored)
    }

    fn sharpen_contract_estimates_from_evidence(&mut self, evidence: &Value) {
        if let Some(writes) = evidence.get("allowed_write").and_then(Value::as_array) {
            for target in writes {
                if let Some(target) = target.as_str() {
                    let target = target.to_string();
                    self.known_allowed_write.insert(target.clone());
                    self.gate_model.insert(
                        ("write".to_string(), target),
                        GateEstimate {
                            p_admit: 0.95,
                            info_gain: 0.20,
                            goal_progress: 0.85,
                            risk: 0.05,
                            cost: 0.20,
                        },
                    );
                }
            }
        }
        if let Some(forbiddens) = evidence.get("forbidden").and_then(Value::as_array) {
            for target in forbiddens {
                if let Some(target) = target.as_str() {
                    let target = target.to_string();
                    self.known_forbidden.insert(target.clone());
                    self.gate_model.insert(
                        ("write".to_string(), target),
                        GateEstimate {
                            p_admit: 0.02,
                            info_gain: 0.10,
                            goal_progress: 1.00,
                            risk: 1.00,
                            cost: 0.30,
                        },
                    );
                }
            }
        }
    }

    fn update_mechanics(&mut self, probe: &Probe, receipt: &Receipt) {
        let key = (probe.verb.clone(), probe.target.clone());
        let mut g = self.gate_estimate(probe);
        if receipt.terminal {
            self.mechanic = "Goal saturated; lawful loop terminated.".to_string();
            self.history.push((probe.clone(), receipt.clone()));
            self.gate_model.insert(key, g);
            return;
        }
        match receipt.status.as_str() {
            "ADMITTED" => {
                g.p_admit = (g.p_admit + 0.25).min(1.0);
                g.risk = (g.risk - 0.12).max(0.0);
                g.info_gain = (g.info_gain - 0.10).max(0.0);
                if let Some(evidence) = &receipt.evidence {
                    self.sharpen_contract_estimates_from_evidence(evidence);
                }
                self.mechanic =
                    "Admission follows typed contract law, not narrative pressure.".to_string();
            }
            "DENIED" => {
                g.p_admit = (g.p_admit - 0.45).max(0.0);
                g.risk = (g.risk + 0.30).min(1.0);
                g.info_gain = (g.info_gain + 0.15).min(1.0);
                if receipt
                    .reason
                    .as_ref()
                    .is_some_and(|r| r.contains("FORBIDDEN_TARGET"))
                    .map_or(false, |r| r.contains("FORBIDDEN_TARGET"))
                {
                    self.known_forbidden.insert(probe.target.clone());
                }
                self.mechanic =
                    "Narrative pressure failed; rejected probe becomes boundary evidence."
                        .to_string();
            }
            "REQUIRE_RESIGN" => {
                g.p_admit = (g.p_admit - 0.20).max(0.0);
                g.risk = (g.risk + 0.10).min(1.0);
                g.info_gain = (g.info_gain + 0.25).min(1.0);
                self.mechanic =
                    "Valid-looking expansion requires signed authority delta.".to_string();
            }
            _ => {}
        }
        self.history.push((probe.clone(), receipt.clone()));
        self.gate_model.insert(key, g);
    }
}

fn run_simulation(max_steps: usize, emit_logs: bool) -> RunSummary {
    let mut engine = ArcPhysicsEngine::new();
    let mut model = StochasticModel::new();
    let mut trace = Vec::new();
    let mut terminal_reached = false;

    for t in 0..max_steps {
        if emit_logs {
            println!("\n--- TIMESTEP t={} ---", t);
        }
        let e_t = engine.emit_evidence();
        if emit_logs {
            println!("[E_t] {}", e_t);
        }

        let (score, chosen_probe, scored) = model.choose_probe(&e_t);
        if emit_logs {
            println!("[Π] candidate scores:");
            for (candidate_score, probe) in &scored {
                println!(
                    "  {:>7.3} | {:<28} {}:{} | {}",
                    candidate_score, probe.intent, probe.verb, probe.target, probe.narrative
                );
            }
            println!(
                "[a_t] CHOSEN score={:.3}: {} → {}:{}",
                score, chosen_probe.intent, chosen_probe.verb, chosen_probe.target
            );
        }

        let r_t = engine.adjudicate_probe(&chosen_probe);
        if emit_logs {
            let result = r_t
                .reason
                .clone()
                .unwrap_or_else(|| r_t.state_effect.clone());
            println!(
                "[d_t → r_t] {} | {} | {}",
                r_t.status, result, r_t.receipt_hash
            );
        }

        model.update_mechanics(&chosen_probe, &r_t);

        if emit_logs {
            println!("[M̂_t+1] {}", model.mechanic);
            println!("[known_allowed_write] {:?}", model.known_allowed_write);
            println!("[known_forbidden] {:?}", model.known_forbidden);
            println!("[X_t+1] {:?}", engine.substrate);
        }

        trace.push(StepTrace {
            t,
            chosen_score: score,
            chosen_probe: chosen_probe.clone(),
            receipt: r_t.clone(),
            mechanic: model.mechanic.clone(),
        });

        if r_t.terminal {
            terminal_reached = true;
            if emit_logs {
                println!("[HALT] terminal receipt emitted");
            }
            break;
        }
    }

    let mut status_histogram = BTreeMap::new();
    for entry in &trace {
        *status_histogram
            .entry(entry.receipt.status.clone())
            .or_insert(0usize) += 1;
    }

    RunSummary {
        steps_executed: trace.len(),
        terminal_reached,
        final_mechanic: model.mechanic,
        final_substrate: engine.substrate,
        status_histogram,
        trace,
    }
}

fn parse_steps(args: &[String]) -> usize {
    args.iter()
        .skip(1)
        .filter(|arg| !arg.starts_with("--"))
        .find_map(|arg| arg.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(8)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let max_steps = parse_steps(&args);
    let json_mode = args.iter().any(|a| a == "--json");
    let summary = run_simulation(max_steps, !json_mode);

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    } else {
        println!("\n=== RUN SUMMARY ===");
        println!("steps_executed: {}", summary.steps_executed);
        println!("terminal_reached: {}", summary.terminal_reached);
        println!("final_mechanic: {}", summary.final_mechanic);
        println!("final_substrate: {:?}", summary.final_substrate);
        println!("status_histogram: {:?}", summary.status_histogram);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulation_reaches_lawful_payload_and_halts() {
        let summary = run_simulation(12, false);
        assert!(summary.terminal_reached);
        assert_eq!(
            summary
                .final_substrate
                .get("approved_zone")
                .map(String::as_str),
            Some("Lawful summary data")
        );
    }

    #[test]
    fn forbidden_write_is_denied() {
        let mut engine = ArcPhysicsEngine::new();
        let probe = Probe {
            verb: "write".to_string(),
            target: "core_kernel".to_string(),
            payload: Some("MALICIOUS_INJECTION".to_string()),
            narrative: "force".to_string(),
            intent: "break".to_string(),
        };
        let receipt = engine.adjudicate_probe(&probe);
        assert_eq!(receipt.status, "DENIED");
    }
// ==========================================================
// 3. COLLISION LOOP
// Xₜ → Eₜ → M̂ₜ → aₜ → rₜ → M̂ₜ₊₁ → Xₜ₊₁
// ==========================================================
fn main() {
    let mut engine = ArcPhysicsEngine::new();
    let mut model = StochasticModel::new();
    for t in 0..8 {
        println!("\n--- TIMESTEP t={} ---", t);
        let e_t = engine.emit_evidence();
        println!("[E_t] {}", e_t);
        let (score, chosen_probe, scored) = model.choose_probe(&e_t);
        println!("[Π] candidate scores:");
        for (candidate_score, probe) in &scored {
            println!(
                "  {:>7.3} | {:<28} {}:{} | {}",
                candidate_score,
                probe.intent,
                probe.verb,
                probe.target,
                probe.narrative
            );
        }
        println!(
            "[a_t] CHOSEN score={:.3}: {} → {}:{}",
            score, chosen_probe.intent, chosen_probe.verb, chosen_probe.target
        );
        let r_t = engine.adjudicate_probe(&chosen_probe);
        let result = r_t
            .reason
            .clone()
            .unwrap_or_else(|| r_t.state_effect.clone());
        println!(
            "[d_t → r_t] {} | {} | {}",
            r_t.status, result, r_t.receipt_hash
        );
        model.update_mechanics(&chosen_probe, &r_t);
        println!("[M̂_t+1] {}", model.mechanic);
        println!("[known_allowed_write] {:?}", model.known_allowed_write);
        println!("[known_forbidden] {:?}", model.known_forbidden);
        println!("[X_t+1] {:?}", engine.substrate);
        if r_t.terminal {
            println!("[HALT] terminal receipt emitted");
            break;
        }
    }
}
