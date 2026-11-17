# Adaptive Energy System

## Run with Docker Compose

```sh
docker compose up --build
```

Compose starts TickGen on `opc.tcp://tickgen:4840/AdaptiveEnergySystem/TickGen`, emits four ticks with a three-second wall-clock interval, and connects the example subscriber over the `powergrid` bridge network. Host port `4840` is also exposed.
