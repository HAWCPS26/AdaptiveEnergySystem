# Adaptive Energy System

Components for simulating an adaptive energy system. All components
communicate over OPC UA in the `urn:AdaptiveEnergySystem` namespace.

## Services

| Service | Endpoint | Purpose |
|---|---|---|
| `tickgen` | `opc.tcp://tickgen:4840/AdaptiveEnergySystem/TickGen` | Advances simulated time in 15-minute ticks |
| `dl` | `opc.tcp://dl:4842/AdaptiveEnergySystem/DeferrableLoad` | Externally controlled deferrable washing machine |
| `example_subscriber` | client only | Prints each TickGen tick |

## Running the simulation

```sh
docker compose up --build
```
