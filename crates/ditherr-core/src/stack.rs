//! The Studio effect stack: pre-process [`Step`]s that can be switched off
//! without losing their settings.
//!
//! The engine itself only ever sees the enabled steps in order
//! ([`enabled_steps`]), so a [`Look`](crate::Look) stays a plain list and a
//! stack with every layer enabled round-trips to the same look.

use serde::{Deserialize, Serialize};

use crate::preprocess::Step;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Disabled layers keep their settings but are skipped by the engine.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    pub step: Step,
}

fn enabled_by_default() -> bool {
    true
}

impl Layer {
    pub fn new(step: Step) -> Self {
        Self {
            enabled: true,
            step,
        }
    }
}

/// Layers for `steps`, all enabled.
pub fn layers_from_steps(steps: &[Step]) -> Vec<Layer> {
    steps.iter().cloned().map(Layer::new).collect()
}

/// The steps the engine runs: enabled layers, in stack order.
pub fn enabled_steps(layers: &[Layer]) -> Vec<Step> {
    layers
        .iter()
        .filter(|l| l.enabled)
        .map(|l| l.step.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_layers_are_skipped_and_order_is_kept() {
        let mut layers =
            layers_from_steps(&[Step::Exposure(0.5), Step::Contrast(0.2), Step::FlipVertical]);
        assert_eq!(enabled_steps(&layers).len(), 3);
        layers[1].enabled = false;
        assert_eq!(
            enabled_steps(&layers),
            vec![Step::Exposure(0.5), Step::FlipVertical]
        );
    }

    #[test]
    fn all_enabled_round_trips() {
        let steps = vec![Step::Gamma(1.2), Step::Saturation(0.5)];
        assert_eq!(enabled_steps(&layers_from_steps(&steps)), steps);
    }
}
