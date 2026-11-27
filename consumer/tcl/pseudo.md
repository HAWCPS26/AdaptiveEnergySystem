```C
CLASS TCLHeatingUnit
// Instance Configuration
    P_nom_kw = 2.0            // Nominal electrical power (kW)
    T_set = 21.0              // User target temperature (°C)
    deadband = 1.0            // Thermostat hysteresis width (°C)
    T_min_safe = 18.0         // Hard lower safety limit (°C)
    T_max_safe = 25.0         // Hard upper safety limit (°C)

// Control State
    heater_on = FALSE
    ems_mode = "NORMAL"       // "NORMAL", "CURTAIL", or "BOOST"
    ems_timer = 0             // Remaining override duration (seconds)

// Electrical Metering State (exposed to OPC UA / EMS)
    current_power_kw = 0.0    // Active load during the current tick (kW)
    energy_tick_kwh = 0.0     // Energy consumed during the current tick (kWh)
    total_energy_kwh = 0.0    // Cumulative energy meter reading (kWh)

// Interface invoked by the EMS (e.g., via OPC UA Method)
    FUNCTION SetEmsMode(mode, duration_seconds):
        ems_mode = mode
        ems_timer = duration_seconds
    END FUNCTION

// Executed once per tick for each device instance
    FUNCTION Step(dt_seconds):
        T_current = ReadTemperatureSensor()

        // Priority 1: Enforce local safety boundaries
        IF T_current <= T_min_safe THEN
            heater_on = TRUE
            ems_mode = "NORMAL"
            ems_timer = 0

        ELSE IF T_current >= T_max_safe THEN
            heater_on = FALSE
            ems_mode = "NORMAL"
            ems_timer = 0

        // Priority 2: Execute active EMS grid command
        ELSE IF ems_timer > 0 THEN
            ems_timer = ems_timer - dt_seconds

            IF ems_mode == "CURTAIL" THEN
                heater_on = FALSE
            ELSE IF ems_mode == "BOOST" THEN
                heater_on = TRUE
            END IF

            IF ems_timer <= 0 THEN
                ems_mode = "NORMAL"
            END IF

        // Priority 3: Standard thermostat hysteresis control
        ELSE
            IF T_current <= T_set - (deadband / 2) THEN
                heater_on = TRUE
            ELSE IF T_current >= T_set + (deadband / 2) THEN
                heater_on = FALSE
            END IF
        END IF

        SetHeatingElement(heater_on)

        // Electricity Consumption Model
        // Estimate power draw based on the nominal power 
        IF heater_on == TRUE THEN
            current_power_kw = P_nom_kw
        ELSE
            current_power_kw = 0.0
        END IF

        energy_tick_kwh = current_power_kw * (dt_seconds / 3600.0)
        total_energy_kwh = total_energy_kwh + energy_tick_kwh

        RETURN current_power_kw, energy_tick_kwh

        // Or read actual electrical power from hardware sensor
        // Additional Class State
        prev_power_kw = 0.0       // Measured power from the previous tick (kW)

        current_power_kw = ReadPowerSensorKw()

        // Integrate real-world power into energy using the trapezoidal rule
        average_power_kw = (prev_power_kw + current_power_kw) / 2.0
        energy_tick_kwh = average_power_kw * (dt_seconds / 3600.0)
        total_energy_kwh = total_energy_kwh + energy_tick_kwh

        // Store current reading for the next tick
        prev_power_kw = current_power_kw

        RETURN current_power_kw, energy_tick_kwh

    END FUNCTION
END CLASS
```

