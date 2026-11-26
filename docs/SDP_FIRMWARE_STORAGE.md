# SDP firmware source and upload history

SynaptaCode owns the source catalog. The board reports `device_id`, `project_id` and `build_id`. The owner can give each device a display name and link it to a local Git workspace and source name. Multiple boards may share a project and build history, with a separate upload history per device.

The owner sets ordered storage folders in SynaptaCode. Use an absolute local path or an SMB path such as `\\server\share\firmware` that the SynaptaCode process can access. The first verified copy serves each build. An unavailable or corrupt copy falls back to the next folder. SynaptaCode preserves the configured order and repairs available mirrors. A change to the list must preserve at least one verified copy of each archived build. The existing folders stay on disk.

Each root contains `projects/<projectId>/builds/<buildId>/<sha256>/source.zip` and `build.json`. The ZIP comes from an exact full Git commit. The build record includes its source name and archive SHA-256. Build automation must build from that archive before uploading. Upload records go under `devices/<deviceId>/uploads/` and refer to the build and the board-reported result. A verified running build, the latest source build, and the most recent upload attempt can differ.

## SynaptaCode REST contract

Routes below use `/api/connect/firmware` and the existing authenticated owner API. IDs are lowercase alphanumeric with optional `-` and `_`, at most 96 characters.

| Method and path | Input or result |
| --- | --- |
| `GET /storage` | `{ paths: string[], activePath: string \| null }`. `activePath` is the first accessible directory. Resolving a build checks its individual copies. |
| `PUT /storage` | `{ paths: string[] }` in priority order. Returns storage status. |
| `POST /storage/sync` | Retries mirrors. Returns `{ paths, activePath, pendingCopies }`. |
| `GET /devices` | Device bindings with `deviceId`, `name`, `projectId`, `sourceName`, `workspace`. |
| `PUT /devices/:deviceId` | `{ name, projectId, sourceName, workspace }`. Returns the binding. |
| `GET /devices/:deviceId` | `{ device, builds, uploads }`. Newest first. |
| `POST /devices/:deviceId/builds` | `{ buildId, commit }`. Archives the Git commit and returns the build metadata. |
| `GET /devices/:deviceId/source` | Optional `?buildId=…`. Returns the archive's verified `archivePath`, `storagePath` and `build`. With no build ID it selects the newest archived source. |
| `POST /devices/:deviceId/uploads` | `{ uploadId, buildId, outcome, runningBuildId? }`. Outcome is `verified`, `failed`, `rolled_back` or `unknown`. A verified result requires `runningBuildId` to match the intended build. |

The build and upload endpoints are for the host integration to call after preparation and after checking the board, respectively. Agent workflows can use the owner API on the PC and request the exact build the board reports.
