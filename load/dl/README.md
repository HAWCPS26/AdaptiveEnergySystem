# Deferrable washing machine

A washing machine that is controlled from the outside over OPC UA.

## Control contract

| Method | Input | Behaviour |
|---|---|---|
| `SetDeadline` | `Deadline: DateTime` | Sets the simulation time by which the cycle must finish. Required before `Start`. |
| `Start` | none | Starts a new cycle from the beginning. Rejected without a deadline or while running/paused. |
| `Pause` | none | Freezes the cycle and stops consuming power. |
| `Resume` | none | Continues a paused cycle from the same point. |
| `Stop` | none | Ends a running or paused cycle and discards its progress. |

There is no automatic start. If no EMS calls `Start`, the machine stays `Idle`.

## Cycle model

| Phase | Duration | Average power | Energy |
|---|---:|---:|---:|
| Fill & pre-wash | 10 min | 0.10 kW | 0.017 kWh |
| Heat & wash | 20 min | 2.00 kW | 0.667 kWh |
| Main wash | 20 min | 0.20 kW | 0.067 kWh |
| Rinse | 15 min | 0.20 kW | 0.050 kWh |
| Drain & spin | 10 min | 0.45 kW | 0.075 kWh |
| Final spin | 15 min | 0.45 kW | 0.113 kWh |
| **Total** | **90 min** | | **0.988 kWh** |

`Pause` freezes both the phase clock and the energy counter. `Stop` resets the
cycle to zero.

## Deadline semantics

`Deadline` is a **finish-by** time in simulation time. `LatestStart` is
`Deadline - 90 minutes`, assuming no pauses. Pauses extend the elapsed
simulation time, so an EMS that plans to pause should start earlier.

## OPC UA nodes

Object `DeferrableLoad`:

| Node | Type | Meaning |
|---|---|---|
| `State` | `String` | `Idle`, `Running`, `Paused`, `Done` or `Stopped` |
| `Phase` | `String` | Most recent wash phase |
| `DeadlineSet` | `Boolean` | Whether a deadline has been configured |
| `Deadline` | `DateTime` | Finish-by time in simulation time |
| `LatestStart` | `DateTime` | Latest start that still finishes before `Deadline` |
| `DeadlineMet` | `Boolean` | Whether the last completed cycle met the deadline |
| `CycleDurationMinutes` | `UInt32` | 90 |
| `CycleElapsedMinutes` | `Double` | Simulated minutes completed in the cycle |
| `CycleRemainingMinutes` | `Double` | Simulated minutes left |
| `ProgressPercent` | `Double` | 0 to 100 |
| `CurrentPowerKw` | `Double` | Average power over the latest tick |
| `EnergyDeliveredKwh` | `Double` | Energy consumed by the current or last cycle |
| `CycleEnergyKwh` | `Double` | Nominal energy of a complete cycle |
| `SimulationTime` | `DateTime` | Last processed simulation time |
| `TickGenConnected` | `Boolean` | Whether the appliance is subscribed to TickGen |

## Sources for the cycle model

- European Commission, *Washing Machines* — the EU label uses the `eco 40-60`
  programme and reports energy per 100 cycles and programme duration.
  <https://energy-efficient-products.ec.europa.eu/product-list/washing-machines_en>
- C-Lec Electrical, “How Much Electric Does a Washing Machine Use?” — a
  90-minute 40 °C cycle with 10 minutes of fill, a 25-minute heat/wash phase at
  1.8–2.2 kW using 0.5–0.8 kWh, then low-power wash/rinses and a spin/drain
  phase; total 0.6–1 kWh.
  <https://c-lecelectrical.co.uk/how-much-electric-does-a-washing-machine-use>
- Pipattanasomporn et al. (2014), “Load Profiles of Selected Major Household
  Appliances and Their Demand Response Opportunities”, *IEEE Transactions on
  Smart Grid* 5(2), 742–750 — one-second measured load profiles for clothes
  washers and other appliances.
  <https://ari.vt.edu/research-data>
