//! Approximate electrical profile of a 40 °C household wash cycle.
//!
//! The phase order and power levels follow a measured-style 90-minute 40 °C
//! cycle: a short fill, a high-power heating phase, a long low-power wash and
//! rinse phase, and a short higher-power drain and spin phase. Heating is the
//! dominant energy user, as reported by Kimmel et al. (2023).
//!
//! Sources used for the shape are documented in `consumer/dl/README.md`.

/// Total simulated duration of the wash cycle.
pub const CYCLE_DURATION_MINUTES: u32 = 90;

/// One phase of the wash cycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Phase {
    pub name: &'static str,
    pub duration_minutes: u32,
    pub power_kw: f64,
}

/// A 90-minute 40 °C daily wash.
pub const PHASES: &[Phase] = &[
    Phase {
        name: "Fill & pre-wash",
        duration_minutes: 10,
        power_kw: 0.10,
    },
    Phase {
        name: "Heat & wash",
        duration_minutes: 20,
        power_kw: 2.00,
    },
    Phase {
        name: "Main wash",
        duration_minutes: 20,
        power_kw: 0.20,
    },
    Phase {
        name: "Rinse",
        duration_minutes: 15,
        power_kw: 0.20,
    },
    Phase {
        name: "Drain & spin",
        duration_minutes: 10,
        power_kw: 0.45,
    },
    Phase {
        name: "Final spin",
        duration_minutes: 15,
        power_kw: 0.45,
    },
];

/// Energy and dominant phase for one tick of the profile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickEnergy {
    pub energy_kwh: f64,
    pub average_power_kw: f64,
    pub phase: &'static str,
    pub minutes: f64,
}

/// Total electrical energy of one complete cycle.
pub fn cycle_energy_kwh() -> f64 {
    PHASES
        .iter()
        .map(|phase| phase.power_kw * f64::from(phase.duration_minutes) / 60.0)
        .sum()
}

/// Return the phase that contains `elapsed_minutes`.
pub fn phase_at(elapsed_minutes: f64) -> &'static Phase {
    let mut cursor = 0.0;
    for phase in PHASES {
        let end = cursor + f64::from(phase.duration_minutes);
        if elapsed_minutes < end {
            return phase;
        }
        cursor = end;
    }
    PHASES.last().expect("the wash profile is not empty")
}

/// Integrate the profile from `start_minutes` for `tick_minutes`.
///
/// The dominant phase is the phase contributing the most energy during the
/// tick. This is what the appliance reports as its current phase.
pub fn tick(start_minutes: f64, tick_minutes: f64) -> TickEnergy {
    let start = start_minutes.clamp(0.0, f64::from(CYCLE_DURATION_MINUTES));
    let end = (start + tick_minutes.max(0.0)).min(f64::from(CYCLE_DURATION_MINUTES));
    let minutes = end - start;
    if minutes <= 0.0 {
        return TickEnergy {
            energy_kwh: 0.0,
            average_power_kw: 0.0,
            phase: phase_at(start).name,
            minutes: 0.0,
        };
    }

    let mut cursor = 0.0;
    let mut energy_kwh = 0.0;
    let mut dominant_energy = 0.0;
    let mut dominant_phase = phase_at(start).name;

    for phase in PHASES {
        let phase_start = cursor;
        let phase_end = cursor + f64::from(phase.duration_minutes);
        let overlap_start = start.max(phase_start);
        let overlap_end = end.min(phase_end);
        if overlap_end > overlap_start {
            let phase_energy = phase.power_kw * (overlap_end - overlap_start) / 60.0;
            energy_kwh += phase_energy;
            if phase_energy > dominant_energy {
                dominant_energy = phase_energy;
                dominant_phase = phase.name;
            }
        }
        cursor = phase_end;
    }

    TickEnergy {
        energy_kwh,
        average_power_kw: energy_kwh / (minutes / 60.0),
        phase: dominant_phase,
        minutes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_is_ninety_minutes() {
        let duration: u32 = PHASES.iter().map(|phase| phase.duration_minutes).sum();
        assert_eq!(duration, CYCLE_DURATION_MINUTES);
    }

    #[test]
    fn cycle_energy_is_about_one_kwh() {
        assert!((cycle_energy_kwh() - 0.9875).abs() < 1e-9);
    }

    #[test]
    fn tick_energy_follows_the_phase_profile() {
        let expected = [0.183_333_33, 0.5, 0.05, 0.05, 0.091_666_67, 0.1125];
        let mut elapsed = 0.0;
        for (tick_index, expected_energy) in expected.iter().enumerate() {
            let result = tick(elapsed, 15.0);
            assert!(
                (result.energy_kwh - expected_energy).abs() < 1e-6,
                "tick {tick_index} energy was {}",
                result.energy_kwh
            );
            elapsed += 15.0;
        }
        assert!((elapsed - f64::from(CYCLE_DURATION_MINUTES)).abs() < 1e-9);
    }

    #[test]
    fn reports_the_dominant_phase_for_each_tick() {
        let expected = [
            "Heat & wash",
            "Heat & wash",
            "Main wash",
            "Rinse",
            "Drain & spin",
            "Final spin",
        ];
        let mut elapsed = 0.0;
        for expected_phase in expected {
            let result = tick(elapsed, 15.0);
            assert_eq!(result.phase, expected_phase);
            elapsed += 15.0;
        }
    }

    #[test]
    fn average_power_matches_energy_over_the_tick() {
        let result = tick(0.0, 15.0);
        assert!((result.average_power_kw - result.energy_kwh / 0.25).abs() < 1e-9);
    }
}
