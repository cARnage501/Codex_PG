use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("sha256:{:x}", h.finalize())
}

pub fn sha256_json<T: Serialize>(v: &T) -> String {
    sha256_bytes(serde_json::to_vec(v).unwrap().as_slice())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainPackage {
    pub manifest: Value,
    pub substrate: BTreeMap<String, Vec<f32>>,
    pub coordinate_law: Value,
    pub region_map: Value,
    pub read_modes: Value,
    pub adapter_abi: Value,
}

impl BrainPackage {
    pub fn demo() -> Self {
        let mut substrate = BTreeMap::new();
        substrate.insert("encoder.layer_0.activation".to_string(), vec![0.1; 64]);
        substrate.insert("encoder.layer_1.activation".to_string(), vec![0.2; 32]);
        substrate.insert("output.logits".to_string(), vec![0.9, 0.1]);
        let coordinate_law = json!({
            "schema": "coordinate_law_v0",
            "coordinate_system": "named_module_region_v0",
            "address_format": "module_path:region_name"
        });
        let region_map = json!({
            "schema": "region_map_v0",
            "brain_id": "example.pattern_brain.v0",
            "regions": [
                {"region_id": "encoder.layer_0.activation", "readable": true, "writable": false, "shape": [64]},
                {"region_id": "encoder.layer_1.activation", "readable": true, "writable": false, "shape": [32]},
                {"region_id": "output.logits", "readable": true, "writable": false, "shape": [2]}
            ]
        });
        let read_modes = json!({"schema":"read_mode_v0","read_mode_id":"readonly_activation_projection_v0","allowed_regions":["encoder.layer_0.activation","encoder.layer_1.activation","output.logits"],"write_back_allowed":false,"gradient_flow_allowed":false,"mutation_allowed":false});
        let adapter_abi = json!({"schema":"adapter_abi_v0","adapter_abi_id":"torch_readonly_projection_adapter_v0"});
        let substrate_hash = sha256_json(&substrate);
        let manifest = json!({
            "schema":"brain_manifest_v0",
            "brain_id":"example.pattern_brain.v0",
            "version":"0.1.0",
            "substrate_hash": substrate_hash,
            "mutation_default":"forbidden",
            "supported_read_modes":["readonly_activation_projection_v0"],
            "supported_adapter_abis":["torch_readonly_projection_adapter_v0"]
        });
        Self {
            manifest,
            substrate,
            coordinate_law,
            region_map,
            read_modes,
            adapter_abi,
        }
    }

    pub fn verify(&self) -> Result<(), String> {
        if self.manifest.get("mutation_default") != Some(&Value::String("forbidden".into())) {
            return Err("mutation_default must be forbidden".into());
        }
        let expected = self
            .manifest
            .get("substrate_hash")
            .and_then(Value::as_str)
            .ok_or("missing substrate_hash")?;
        let actual = sha256_json(&self.substrate);
        if expected != actual {
            return Err("brain hash mismatch".into());
        }
        Ok(())
    }

    pub fn hash(&self) -> String {
        sha256_json(&self.substrate)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingSession {
    pub binding_id: String,
    pub brain_id: String,
    pub brain_hash: String,
    pub adapter_id: String,
    pub read_mode_id: String,
    pub allowed_regions: Vec<String>,
    pub mutation_allowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadOnlyProjectionAdapter {
    pub adapter_id: String,
}

impl ReadOnlyProjectionAdapter {
    pub fn new() -> Self {
        Self {
            adapter_id: "readonly_projection_adapter.v0".into(),
        }
    }
    pub fn bind(&self, brain: &BrainPackage, read_mode: &str) -> Result<BindingSession, String> {
        brain.verify()?;
        if read_mode != "readonly_activation_projection_v0" {
            return Err("READ_REJECTED".into());
        }
        Ok(BindingSession {
            binding_id: "bind_0001".into(),
            brain_id: brain.manifest["brain_id"].as_str().unwrap().into(),
            brain_hash: brain.hash(),
            adapter_id: self.adapter_id.clone(),
            read_mode_id: read_mode.into(),
            allowed_regions: vec![
                "encoder.layer_0.activation".into(),
                "encoder.layer_1.activation".into(),
                "output.logits".into(),
            ],
            mutation_allowed: false,
        })
    }
    pub fn read_region(
        &self,
        binding: &BindingSession,
        brain: &BrainPackage,
        region: &str,
    ) -> Result<Vec<f32>, String> {
        if !binding.allowed_regions.contains(&region.to_string()) {
            return Err("REJECT_ROUTE".into());
        }
        brain
            .substrate
            .get(region)
            .cloned()
            .ok_or("REJECT_ROUTE".into())
    }
    pub fn write_region(&self, _region: &str, _value: Vec<f32>) -> Result<(), String> {
        Err("REJECT_ADAPTER_WRITE".into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingBody {
    pub columns: usize,
    pub state: BTreeMap<String, String>,
}
impl WorkingBody {
    pub fn new() -> Self {
        Self {
            columns: 1,
            state: BTreeMap::new(),
        }
    }
    pub fn hash(&self) -> String {
        sha256_json(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpansionDelta {
    pub reason: String,
    pub previous_working_body_hash: String,
    pub next_working_body_hash: String,
    pub brain_modules_modified: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraversalResult {
    pub output: usize,
    pub receipt: Value,
}

pub fn run_demo_traversal() -> Result<TraversalResult, String> {
    let brain = BrainPackage::demo();
    let adapter = ReadOnlyProjectionAdapter::new();
    let binding = adapter.bind(&brain, "readonly_activation_projection_v0")?;
    let mut wb = WorkingBody::new();
    let wb_before = wb.hash();
    let b_before = brain.hash();

    let r0 = adapter.read_region(&binding, &brain, "encoder.layer_0.activation")?;
    let r1 = adapter.read_region(&binding, &brain, "encoder.layer_1.activation")?;
    let sum: f32 = r0.iter().chain(r1.iter()).copied().sum();
    let confidence = (sum / 100.0).min(1.0);

    let mut expansion_hashes = Vec::new();
    if confidence < 0.5 {
        wb.columns += 1;
        wb.state.insert("expanded".into(), "true".into());
        let d = ExpansionDelta {
            reason: "capacity_rejection".into(),
            previous_working_body_hash: wb_before.clone(),
            next_working_body_hash: wb.hash(),
            brain_modules_modified: vec![],
        };
        expansion_hashes.push(sha256_json(&d));
    }

    let logits = adapter.read_region(&binding, &brain, "output.logits")?;
    let output = if logits[0] >= logits[1] { 0 } else { 1 };
    let out_hash = sha256_json(&logits);
    let route = json!({"steps":[{"event":"read_region","region_id":"encoder.layer_0.activation"},{"event":"read_region","region_id":"encoder.layer_1.activation"},{"event":"compose","target":"working_body.column_0.composer"},{"event":"emit_candidate_output","target":"output_head"}]});
    let route_hash = sha256_json(&route);
    let b_after = brain.hash();
    let receipt = json!({
        "schema":"traversal_receipt_v0",
        "brain_hash_before": b_before,
        "brain_hash_after": b_after,
        "immutable_substrate_verified": b_before == b_after,
        "adapter_id": binding.adapter_id,
        "read_mode_id": binding.read_mode_id,
        "route_hash": route_hash,
        "output_hash": out_hash,
        "output_claim_status":"proposal_only",
        "expansion_delta_hashes": expansion_hashes,
        "working_body_before_hash": wb_before,
        "working_body_after_hash": wb.hash()
    });
    Ok(TraversalResult { output, receipt })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn brain_hash_stable_during_read() {
        let r = run_demo_traversal().unwrap();
        assert!(r.receipt["immutable_substrate_verified"].as_bool().unwrap());
    }
    #[test]
    fn adapter_cannot_write_back() {
        let a = ReadOnlyProjectionAdapter::new();
        assert_eq!(
            a.write_region("x", vec![]).unwrap_err(),
            "REJECT_ADAPTER_WRITE"
        );
    }
    #[test]
    fn illegal_region_access_rejected() {
        let brain = BrainPackage::demo();
        let a = ReadOnlyProjectionAdapter::new();
        let b = a.bind(&brain, "readonly_activation_projection_v0").unwrap();
        assert_eq!(
            a.read_region(&b, &brain, "secret.undeclared.region")
                .unwrap_err(),
            "REJECT_ROUTE"
        );
    }
}
