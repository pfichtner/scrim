use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    NotFound,
}

#[derive(Debug)]
pub struct HealthCheckResult {
    pub status: HealthStatus,
    pub message: String,
}

impl HealthCheckResult {
    pub fn healthy() -> Self {
        Self {
            status: HealthStatus::Healthy,
            message: "FUSE mount is healthy".to_string(),
        }
    }

    pub fn unhealthy(msg: String) -> Self {
        Self {
            status: HealthStatus::Unhealthy,
            message: msg,
        }
    }

    pub fn not_found() -> Self {
        Self {
            status: HealthStatus::NotFound,
            message: "Mountpoint not found".to_string(),
        }
    }
}

pub fn health_check(mountpoint: &Path) -> HealthCheckResult {
    if !mountpoint.exists() {
        return HealthCheckResult::not_found();
    }
    if !crate::mount::is_mounted(mountpoint) {
        return HealthCheckResult::unhealthy(format!(
            "{} is not a mounted filesystem",
            mountpoint.display()
        ));
    }
    match std::fs::read_dir(mountpoint) {
        Ok(_) => HealthCheckResult::healthy(),
        Err(error) => HealthCheckResult::unhealthy(format!(
            "filesystem at {} is not readable: {error}",
            mountpoint.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_healthy_status() {
        let result = HealthCheckResult::healthy();
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_unhealthy_status() {
        let result = HealthCheckResult::unhealthy("test error".to_string());
        assert_eq!(result.status, HealthStatus::Unhealthy);
    }

    #[test]
    fn test_not_found_status() {
        let result = HealthCheckResult::not_found();
        assert_eq!(result.status, HealthStatus::NotFound);
    }

    #[test]
    fn missing_mountpoint_is_not_found() {
        let result = health_check(Path::new("/nonexistent/scrim"));
        assert_eq!(result.status, HealthStatus::NotFound);
    }
}
