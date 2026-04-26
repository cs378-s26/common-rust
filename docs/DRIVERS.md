# Drivers

This document is for driver developers. We briefly describe the interface that
the Rernel provides to driver developers, along with special considerations when
developing for the Rernel.

## Setup

Drivers are expected to provide an implementation of trait `DeviceDiscovery`. 
All setup work should happen in `am_i_this`, which is currently invoked on startup
when the kernel detects a new device.

