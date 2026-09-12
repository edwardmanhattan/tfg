# ADR-0001: Game objects render as GPUI overlays, never maplibre layers

## Status

Accepted.

## Context

The map viewport (`maplibre-native-rs`) and the command-center UI (GPUI)
share one window. Ships (markers, trails, future game objects) must appear
on top of the map. Two homes were possible: maplibre style layers
(symbol/circle/GeoJSON sources inside the map engine) or GPUI overlay
elements positioned over the rendered frame.

## Decision

Game objects live exclusively in GPUI overlay space. MapLibre stays a pure
map engine underneath: tiles, camera, and nothing else. Client code maps
WGS84 positions to screen pixels (WebMercator, same zoom/center as the map
camera) and draws markers with GPUI elements.

## Consequences

- The projection helper is a sanctioned seam, not prototype debt: any camera
  change must flow to both the map renderer and the overlay projection.
- No map-style manipulation for game state; no GeoJSON sources for ships.
- Follow-to-recenter and live movement arrive with the continuous render
  loop in the build phase, reusing this seam.
