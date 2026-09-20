# Scripting ACIS bodies through cadkernel: findings and plan

Investigation of the blocker on **Solid3D, Body, Region and Surface**
(`docs/plugin-host-model-coverage-ledger.md`). The blocker said a script can
neither produce a valid ACIS payload nor edit one, and that the handover
forbids rewriting an opaque payload without a proven lossless path. This note
records what OCS's own kernel path can already do, measured with two spike tests
(`spike_kernel_body_round_trip`, `spike_kernel_timings` in
`src/app/plugin_host.rs`, both `#[ignore]` because they are slow in a debug
build). Measured on cadcodec `5b682ed` / cadkernel `6f046af`, debug build.

**Conclusion:** the blocker was overstated. OCS already owns a kernel-backed
create/transform/boolean pipeline whose output is verified lossless before it is
accepted. The missing piece is not geometry but an *operation channel* from a
script to that pipeline. Solid3D and Body are then straightforward; Region and
Surface need a little more; the one real cost is boolean speed.

## What exists

| Capability | Where | Notes |
|---|---|---|
| Primitives: box, wedge, cylinder, cone/frustum, elliptical cylinder, sphere, torus, pyramid, pyramid frustum | `scene/model/solid_model.rs` (`box_solid`, ...) | return a kernel `Body` |
| Transforms: place, turn about an axis, mirror, column-major 4x4 | `placed`, `turned`, `mirrored`, `by_matrix` | analytic surfaces move with their frames, no point rewriting |
| Booleans: union, subtract, intersect | `boolean_result` | refuses with a reason instead of returning a broken solid |
| Body to entity payload | `acis_export::solid_to_sat` | **self-verifying**: it lifts its own SAT text and SAB binary back and returns `None` unless both are lossless (`loss.is_empty()`) and validate |
| Entity payload to body | `solid3d_tess::kernel_body` / `kernel_region_body` / `kernel_surface_body` / `kernel_acis_body` | returns `None` when the lift is lossy or has more than one body |
| Regenerating derived data | `app/model_ops.rs::entity_with_boolean_body` | rewrites `acis_data`, `wires`, clears `silhouettes` and `history_handle` for Solid3D, Region and Surface |
| Volume, extent, edge wires | `volume`, `extent`, `edge_wires` | used as oracles below |

`kernel_acis_body` is exactly the "proven lossless path" the handover asked for:
a payload is edited only if it lifts with no loss, otherwise it is left alone.

## Spike results

Everything below passed, with the kernel's own volume as the oracle.

| Test | Result |
|---|---|
| Box 10x6x4 to Solid3D | volume 240, 12 edge wires, lifts back |
| DWG save/reopen | lifts back, volume 240, 12 wires kept. `acis_data` changes from SAT text to **SAB binary** (same solid, different bytes) |
| DXF save/reopen | lifts back, volume 240. Cached `wires` are dropped (0) and are regenerated on demand; silhouettes were already empty |
| Translate by (5,5,5) | volume 240, extent shifted exactly, lifts back |
| Rotate 45 degrees about Z | volume 240, extent -5.657..5.657, lifts back |
| Mirror in X | volume 240, lifts back |
| Box union cylinder | volume 315.277, 16 wires, lifts back |
| Box subtract cylinder | volume 189.815, 14 wires, lifts back |
| Box intersect cylinder | volume 50.185, 2 wires, lifts back |
| Sphere, torus, wedge, pyramid | volumes 112.696, 98.415, 12.000, 28.532; all lift back |
| Same payload in a `Body` entity | lifts back; still lifts back after DWG and after DXF |

Timings (debug build): primitive, transform, `solid_to_sat` and `edge_wires` are
effectively instant (under 0.05 s). **Each boolean took about 23 s**, most likely
a debug-build artifact for a kernel doing heavy numeric work, but it has not been
measured in release and must be before booleans are exposed.

Two things to understand about "lossless":

- DWG rewrites SAT text as SAB binary. The solid is identical (same volume,
  topology, faces) but the bytes are not, so the honest oracle is "lifts back with
  no loss and the same volume", not byte equality. An untouched body that a
  script never edits is written exactly as OCS writes any other body.
- DXF does not carry the cached wires; they are derived data.

## Design

The obstacle is the channel. The generic entity path moves an acadrust
`EntityType` from the plugin to the host; the plugin has no kernel and must never
see or rewrite ACIS bytes. Three ways to give a script the operations:

**A. Host-side normalization with a request carrier (no ABI change).** A
hand-written converter (as for Dimension) turns `create_entity('Solid3D',
primitive='box', ...)` into a Solid3D that carries the request in an otherwise
unused field, and the host's `normalize_scripted_entity` replaces it with the real
payload, exactly as it already does for MLine geometry, Helix splines and image
definitions. It works inside today's v7 protocol, but the carrier field is a hack.

**B. New host operation (recommended).** Add one additive operation to `HostApi`
and the v4 IPC protocol, for example `solid_operation(op) -> Result<Handle, String>`,
with `op` one of: create primitive, transform, boolean, and (later) region from
curves. The host runs it through the pipeline above inside an undo step and
returns the new or updated handle. Because it is a new method with a default
"unsupported" implementation it is backward compatible; bump the minor/negotiate
by capability rather than the major version. Python side:

```python
box  = doc.solids.box(center=(0, 0, 0), size=(10, 6, 4))
cyl  = doc.solids.cylinder(base=(0, 0, -3), radius=2, height=10)
hole = box.subtract(cyl)            # new Solid3D; operands untouched
box.transform(translate=(5, 5, 5))  # in-place, transactional, undoable
```

**C. Do nothing but layer edits** (the current state).

Recommended: **B**, with `solid_operation` covering create, transform and boolean.
Every operation goes through `solid_to_sat` (self-verifying) and the kernel's own
refusal path, so a failure is an error before any mutation, and an untouched
payload is never rewritten. In-place edits of an existing entity are gated by
`kernel_acis_body` returning `Some`: a payload the kernel cannot lift losslessly
is refused with a clear message instead of being rewritten.

## Per-kind feasibility

| Kind | Create | Edit | Notes |
|---|---|---|---|
| **Solid3D** | 9 primitives, boolean of two solids | transform, boolean | Straightforward. Regenerate `wires`; clear `silhouettes`; `history_handle` is cleared by `entity_with_boolean_body`, so a history-carrying solid needs a decision (refuse, or accept losing parametric history). |
| **Body** | same payload container | transform | Same pipeline, `kernel_acis_body` works on a `Body` payload (spike). A true non-solid (sheet or wire) body may not lift; those stay layer-only. |
| **Region** | from closed curve entities (the REGION command's `add_region_model`) | transform | Needs a "region from these curve handles" operation and planar-face construction. |
| **Surface** | plane, extruded, revolved from a profile | transform | The most involved: surface kinds carry `surface_data` besides `acis_data`, and only `Plane` keeps its kind on regeneration (`SurfaceKind::Generic` otherwise). |

## Risks and open questions

1. **Boolean speed.** About 23 s each in a debug build. Time them in release
   before deciding whether they can run on the host thread (the GUI BOOLEAN
   command has the same cost) or need a background task. Everything else is
   instant.
2. **Interoperability cannot be verified here.** `solid_to_sat` is verified by
   OCS's own lifter. Whether AutoCAD accepts the SAT/SAB that OCS writes is not
   testable without AutoCAD, so completion should be scoped as "OCS round trip",
   as for RasterImage.
3. **Parametric history.** Regenerating a payload clears `history_handle`.
4. **Frames.** Some solids carry a body-to-world transform in the SAT
   (`body_transform` in `kernel_acis_body`); the lift already applies it, and the
   spike's translated/rotated bodies confirm it, but a foreign DWG with a
   scaled or sheared body transform should be added as a fixture.
5. **API surface.** `solid_operation` is a protocol addition; decide whether it is
   one generic operation enum or separate methods before starting.

## Suggested phases

1. `solid_operation` protocol and host executor for **create primitive** and
   **transform** on Solid3D, with the full lifecycle test through the real runner
   (volume oracle, DWG and DXF reopen, undo, refusal of a lossy payload).
2. **Booleans** (after release-build timing) and the **Body** container.
3. **Region** from curve handles.
4. **Surface** (plane first).

Phase 1 alone would move Solid3D from blocked to complete-in-OCS.

**Status (2026-09-20): phase 1 is done.** `HostApi::solid_operation` (create primitive and rigid transform), the `doc.solids` Python API and `audit_python_solid3d_lifecycle_over_real_ipc` are in; Solid3D is complete, Body moves through the same channel. Design B was implemented as recommended. Region creation from a closed profile followed (`RegionFromProfile`). Booleans and Surface remain.
