# Adaptive Energy System

Components for simulating an adaptive energy system. All components
communicate over OPC UA in the `urn:AdaptiveEnergySystem` namespace. The
deferrable load is externally controlled; the market and energy management
system are out of scope for now.

## Services

| Service | Endpoint | Purpose |
|---|---|---|
| `tickgen` | `opc.tcp://tickgen:4840/AdaptiveEnergySystem/TickGen` | Advances simulated time in 15-minute ticks |
| `dl` | `opc.tcp://dl:4842/AdaptiveEnergySystem/DeferrableLoad` | Externally controlled deferrable washing machine |
| `example_subscriber` | client only | Prints each TickGen tick |

The washing machine does **not** read prices and does **not** start itself. It
exposes `SetDeadline`, `Start`, `Pause`, `Resume` and `Stop` over OPC UA. A
future energy management system will use those methods to decide when to run
the machine.

## Running the simulation

```sh
docker compose up --build
```
