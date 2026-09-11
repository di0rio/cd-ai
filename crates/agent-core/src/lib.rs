use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
}

pub fn app_info() -> AppInfo {
    AppInfo {
        name: "Cauã AI",
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_uses_crate_version() {
        let info = app_info();
        assert_eq!(info.name, "Cauã AI");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }
}
