# Local automation API

External tools can use EclipseDesktopStation to query robot status and manage
OpMode lifecycles. The API shares the GUI's Robocol connection, including its
robot keepalive traffic and status updates. It can support test runners,
calibration tools, and other local automation without implementing Robocol again.

## Transport

The app exposes a Unix-domain socket while running:

| Platform | Socket |
| --- | --- |
| macOS | `~/Library/Application Support/EclipseDesktopStation/automation.sock` |
| Linux | `~/.local/share/EclipseDesktopStation/automation.sock` |

The socket is owned by the current user with permissions 0600. The endpoint is
not currently enabled on Windows. Launch the app before connecting.

Open one connection per request. Send a newline-terminated UTF-8 JSON object
under 8192 bytes; read one newline-terminated JSON response, then close the
connection. Protocol version is 1. No additional dependencies are required.

## Actions

| Action | Parameters | Behavior |
| --- | --- | --- |
| `status` | None | Returns the current robot snapshot, including when disconnected. |
| `connect` | `host` | Connects to a Robot Controller by IP; reuses the GUI connection to that same robot. |
| `init` | `opmode` | Requests initialization of a registered OpMode. |
| `start` | `opmode` | Requests START for the same OpMode that acknowledged INIT. |
| `stop` | None | Requests the SDK's default stop OpMode. |

Example requests (replace `ExampleTeleOp` with a name from `snapshot.op_modes`):

```json
{"action":"status"}
{"action":"connect","host":"192.168.43.1"}
{"action":"init","opmode":"ExampleTeleOp"}
{"action":"start","opmode":"ExampleTeleOp"}
{"action":"stop"}
```

Successful responses have `ok: true`, `protocol_version: 1`, and `snapshot`.
Errors have `ok: false` and an `error` string. For example:

```json
{"ok":false,"error":"not connected"}
```

The snapshot is the same structure used by the GUI. Useful fields include
`connected`, `peer`, `peer_conflict`, `op_modes`, `active_op_mode`,
`op_mode_running`, `robot_error`, `robot_state`, and `robot_battery`.

## Lifecycle acknowledgments

A successful command response means the request was sent. Poll `status` to
wait for the Robot Controller's acknowledgment:

1. After `connect`, wait for `connected: true`, no `peer_conflict`, and the desired
   name in `op_modes` before requesting INIT.
2. After `init`, wait for `active_op_mode` to equal the requested name and
   `op_mode_running: false`.
3. After `start`, wait for that same name and `op_mode_running: true`.
4. After `stop`, wait for `active_op_mode: "$Stop$Robot$"`. Its running flag may
   remain true because the SDK runs that default OpMode internally.

Missing, empty, and unknown OpMode names are rejected. Use the `stop` action
instead of initializing or starting `$Stop$Robot$` directly. Check `robot_error`
while waiting, and use bounded timeouts so a missing acknowledgment does not
leave the caller waiting indefinitely.

Only one Driver Station can own the robot connection. Peer conflicts are
reported. Connecting to a different robot does not replace an existing GUI
connection; disconnect that connection in the app first.

## Python status example

```python
import json
import socket
from pathlib import Path

path = Path.home() / "Library/Application Support/EclipseDesktopStation/automation.sock"
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
    connection.settimeout(3)
    connection.connect(str(path))
    connection.sendall(b'{"action":"status"}\n')
    with connection.makefile() as source:
        response = json.loads(source.readline())
if not response["ok"]:
    raise RuntimeError(response["error"])
print(response["snapshot"])
```

Use the Linux socket path on Linux. Application-specific measurements and device
commands belong in the robot OpMode or the client tool. This API provides robot
status and lifecycle control. The app's Robocol keepalive remains active while
connected; automation clients should request STOP on completion or error and
implement any device-command timeout in their robot code.
