# drmem-drv-hue

This driver controls Philips Hue devices through a Hue bridge using the CLIP v2 API.

## Architecture

The driver uses a **polling-based** architecture rather than event streaming:
- Periodically polls the Hue bridge to sync device states (default: every 5 seconds)
- Immediately polls a device after sending a command to it
- Simple, reliable, and handles concurrent external changes (e.g., via Hue app)

## Supported Device Types

- **Switch** - On/off control with indicator
- **Dimmer/Bulb** - On/off and brightness control (0-100%) with indicator
- **ColorBulb** - Full RGB color control and brightness
- **Group** - Controls Hue groups/rooms (uses ColorBulb interface)

## Configuration

### Driver-Level Parameters

- `host` (required) - IP address or hostname of the Hue bridge
- `app_id` (required) - Hue application key for authentication
- `poll_interval_secs` (optional) - Polling interval in seconds (default: 5)
- `devices` (required) - Array of device configurations

### Per-Device Parameters

Each device in the `devices` array requires:

- `subpath` (required) - Device path in DrMem hierarchy
- `id` (required) - Hue bridge resource ID (UUID)
- `type` (required) - Device type: `switch`, `dimmer`, `bulb`, `colorbulb`, or `group`
- `override_timeout` (optional) - Timeout in **seconds** for overridable devices
- `poll_on_change` (optional) - Poll immediately after sending command (default: true)

### Example Configuration

```toml
[[driver]]
name = "hue"

[driver.cfg]
host = "192.168.1.100"
app_id = "your-hue-application-key-here"
poll_interval_secs = 5

[[driver.cfg.devices]]
subpath = "living-room/ceiling"
id = "12345678-1234-1234-1234-123456789abc"
type = "colorbulb"
override_timeout = 60  # 60 seconds

[[driver.cfg.devices]]
subpath = "bedroom/lamp"
id = "87654321-4321-4321-4321-cba987654321"
type = "dimmer"

[[driver.cfg.devices]]
subpath = "kitchen/lights"
id = "abcdef12-ab12-ab12-ab12-abcdef123456"
type = "group"
```

## Finding Device IDs

To discover Hue device IDs, you can query the Hue bridge API directly:

```bash
# Get all lights
curl -k -H "hue-application-key: YOUR-APP-KEY" \
  https://YOUR-BRIDGE-IP/clip/v2/resource/light

# Get all groups
curl -k -H "hue-application-key: YOUR-APP-KEY" \
  https://YOUR-BRIDGE-IP/clip/v2/resource/grouped_light
```

Each resource will have an `id` field (UUID format) that you should use in the configuration.

## Authentication

You need to create a Hue application key before using this driver. Follow the official Philips Hue documentation to generate an application key using the bridge's button-press pairing process.

## Behavior

### Polling

- The driver polls all configured devices at the configured interval
- After sending a command to a device, the driver immediately polls that specific device
- This ensures state synchronization without waiting for the next periodic poll

### State Updates

- Brightness values are clamped to 0-100 range
- A brightness of 0 turns the device off
- For color bulbs, colors are converted between RGB and CIE XY color space
- Missing brightness values when a device is "on" default to 100%

### Override Timeout

When `override_timeout` is configured for a device, the device becomes "overridable":

- DrMem will apply settings to the device as normal
- If the device is changed externally (e.g., via the Hue app), DrMem detects this during polling
- The device enters "Override" mode and DrMem stops controlling it
- After the timeout expires (in seconds), DrMem automatically re-applies its last setting
- The timeout is reset if the external override value changes again

This allows temporary manual control without permanently losing DrMem's automation. The `tokio::select!` in the driver's run loop races between periodic polling and override timeout expiration, so the setting will be re-applied within one poll interval after the timeout expires (default: within 5 seconds).

### Error Handling

- Initial sync errors are logged but don't prevent driver startup
- Periodic poll failures are logged but don't crash the driver
- Individual device errors don't affect other devices
- HTTP errors (401, 403, 404) are logged with details

## Color Conversion

For ColorBulb devices, the driver automatically converts between:
- RGB (0-255 per channel) used by DrMem
- CIE XY color space used by Hue devices

This conversion uses the `palette` crate for accurate color representation.