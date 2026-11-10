use std::path::PathBuf;

/// Health check result
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HealthStatus {
    /// FUSE mount is healthy and operational
    Healthy,
    /// FUSE mount is not running or has errored
    Unhealthy,
    /// Mountpoint does not exist
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

/// Perform a health check on the FUSE mount
pub async fn health_check(mountpoint: PathBuf) -> anyhow::Result<HealthCheckResult> {
    if !mountpoint.exists() {
        return Ok(HealthCheckResult::not_found());
    }

    // Check if the FUSE filesystem is accessible
    // This will be expanded to actually check the FUSE device

    Ok(HealthCheckResult::healthy())
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
}
