/// Hue API resource type constants
pub const LIGHT_RESOURCE: &str = "light";
pub const GROUPED_LIGHT_RESOURCE: &str = "grouped_light";

/// Base API path for Hue CLIP v2
pub const API_BASE_PATH: &str = "/clip/v2/resource";

/// Construct a resource URL for a specific type
pub fn resource_url(host: &str, resource_type: &str) -> String {
    format!("https://{}{}/{}", host, API_BASE_PATH, resource_type)
}

/// Construct a specific device URL
pub fn device_url(host: &str, resource_type: &str, id: &str) -> String {
    format!("https://{}{}/{}/{}", host, API_BASE_PATH, resource_type, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_url() {
        assert_eq!(
            resource_url("192.168.1.1", LIGHT_RESOURCE),
            "https://192.168.1.1/clip/v2/resource/light"
        );
    }

    #[test]
    fn test_device_url() {
        assert_eq!(
            device_url("192.168.1.1", LIGHT_RESOURCE, "abc123"),
            "https://192.168.1.1/clip/v2/resource/light/abc123"
        );
    }
}
