# TickGen

Namespace: `urn:AdaptiveEnergySystem`

All nodes are components of `Objects/TickGenerator`:

| Node | OPC UA type | Meaning |
|---|---|---|
| `CurrentTick` | `UInt64` | Latest emitted tick; zero means no tick has been emitted |
| `SimulationTime` | `DateTime` | Simulation time associated with the latest tick |
| `Running` | `Boolean` | Whether ticks are being generated |
| `RemainingTicks` | `UInt64` | Ticks left, including the latest emitted tick |
| `StartTime` | `DateTime` | Simulation time of tick one |
| `TickIntervalSeconds` | `Double` | Wall-clock duration between ticks |
| `SimulatedTickMinutes` | `UInt32` | Simulated duration between ticks; always 15 |
| `TimeSinceLastTick` | `Duration` | Wall-clock **milliseconds** since the latest emitted tick; zero before tick one |

`Start(N)` emits tick one immediately, then emits subsequent ticks at the configured wall-clock interval. Set `TICKGEN_START_DELAY_SECONDS` to delay the automatic start so clients can connect first. Each new tick advances `SimulationTime` by 15 simulated minutes. The run stops one interval after its final tick, without emitting an extra tick. Starting a later run continues the tick number.

`TimeSinceLastTick` is evaluated when read or sampled, so its value continues to increase between ticks. Pause does not stop this elapsed-time measurement.

Agents should monitor `CurrentTick` and then read the other values to react. Its initial monitored-item value reflects the current server state and is not necessarily a newly emitted tick.

### Methods

- `Start(TicksToRun: UInt64)` — start a new finite run and immediately emit its first tick. The count must be greater than zero. Use `Resume` for a paused run.
- `Pause()` — pause without changing the current tick or remaining tick count.
- `Resume()` — continue a paused run without emitting an extra tick.
- `Reset(StartTime: DateTime)` — stop and reset to tick zero.
- `SetTickInterval(Seconds: Double)` — change the wall-clock interval; the new value applies to the next tick wait.

## Checks

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
