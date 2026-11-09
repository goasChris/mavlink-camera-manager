use paperclip::actix::Apiv2Schema;
use serde::{Deserialize, Serialize};

use crate::controls::{
    onvif::{camera::OnvifDeviceInformation, manager::Manager as OnvifManager},
    types::Control,
};

use super::{
    types::*,
    video_source::{VideoSource, VideoSourceAvailable, VideoSourceFormats},
};

#[derive(Apiv2Schema, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VideoSourceOnvifType {
    Onvif(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoSourceOnvif {
    pub name: String,
    pub source: VideoSourceOnvifType,
    #[serde(flatten)]
    pub device_information: OnvifDeviceInformation,
    /// Resolved at runtime; persisted sources use UUID and profile token instead.
    #[serde(skip)]
    pub stream_uri: Option<url::Url>,
}

impl VideoSourceOnvif {
    pub fn matches_uri(&self, source: &str) -> bool {
        let (Some(mut requested), Some(mut current)) =
            (url::Url::parse(source).ok(), self.stream_uri.clone())
        else {
            return false;
        };
        for uri in [&mut requested, &mut current] {
            let _ = uri.set_username("");
            let _ = uri.set_password(None);
        }
        requested == current
    }

    pub async fn resolve(&self) -> anyhow::Result<Self> {
        let mut source = self.clone();
        if self.source_string().starts_with("onvif://") {
            source.stream_uri = Some(
                OnvifManager::get_stream_information(self.source_string())
                    .await?
                    .stream_uri,
            );
        } else {
            if self.source_string().starts_with("rtsp://")
                && let Some(discovered) = OnvifManager::streams_available()
                    .await
                    .into_iter()
                    .find_map(|candidate| match candidate {
                        VideoSourceType::Onvif(candidate)
                            if candidate.matches_uri(self.source_string()) =>
                        {
                            Some(candidate)
                        }
                        _ => None,
                    })
            {
                return Ok(discovered);
            }
            source.stream_uri = Some(url::Url::parse(self.source_string())?);
        }
        Ok(source)
    }

    pub fn explicit_rtsp(source: &str) -> Option<Self> {
        let uri = url::Url::parse(source).ok()?;
        if uri.scheme() != "rtsp" || uri.host_str().is_none() {
            return None;
        }
        Some(Self {
            name: uri.host_str()?.to_string(),
            source: VideoSourceOnvifType::Onvif(source.to_string()),
            device_information: OnvifDeviceInformation {
                manufacturer: String::new(),
                model: String::new(),
                firmware_version: String::new(),
                serial_number: String::new(),
                hardware_id: String::new(),
            },
            stream_uri: Some(uri),
        })
    }
}

impl VideoSourceFormats for VideoSourceOnvif {
    async fn formats(&self) -> Vec<Format> {
        let Ok(source) = self.resolve().await else {
            return vec![];
        };
        let VideoSourceOnvifType::Onvif(stream_uri) = &source.source;
        OnvifManager::get_formats(stream_uri)
            .await
            .unwrap_or_default()
    }
}

impl VideoSource for VideoSourceOnvif {
    fn name(&self) -> &String {
        &self.name
    }

    fn source_string(&self) -> &str {
        match &self.source {
            VideoSourceOnvifType::Onvif(url) => url.as_str(),
        }
    }

    fn set_control_by_name(&self, _control_name: &str, _value: i64) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Onvif source doesn't have controls.",
        ))
    }

    fn set_control_by_id(&self, _control_id: u64, _value: i64) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Onvif source doesn't have controls.",
        ))
    }

    fn control_value_by_name(&self, _control_name: &str) -> std::io::Result<i64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Onvif source doesn't have controls.",
        ))
    }

    fn control_value_by_id(&self, _control_id: u64) -> std::io::Result<i64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Onvif source doesn't have controls.",
        ))
    }

    fn controls(&self) -> Vec<Control> {
        vec![]
    }

    fn is_valid(&self) -> bool {
        match &self.source {
            VideoSourceOnvifType::Onvif(_) => true,
        }
    }

    fn is_shareable(&self) -> bool {
        true
    }
}

impl VideoSourceAvailable for VideoSourceOnvif {
    async fn cameras_available() -> Vec<VideoSourceType> {
        OnvifManager::streams_available().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onvif_explicit_hostname_preserves_url_and_credentials() {
        let uri = "rtsp://user:password@camera.example.com:554/media/1/1/Profile1";
        let source = VideoSourceOnvif::explicit_rtsp(uri).unwrap();
        assert_eq!(source.source_string(), uri);
        assert_eq!(
            source.stream_uri.as_ref().unwrap().host_str(),
            Some("camera.example.com")
        );
        assert_eq!(
            source.stream_uri.as_ref().unwrap().password(),
            Some("password")
        );
        assert!(VideoSourceOnvif::explicit_rtsp("http://camera.example.com").is_none());
        assert!(VideoSourceOnvif::explicit_rtsp("rtsp:///").is_none());
    }

    #[test]
    fn onvif_legacy_matching_ignores_credentials_but_not_profile_or_address() {
        let source =
            VideoSourceOnvif::explicit_rtsp("rtsp://user:password@192.168.1.10/Profile1").unwrap();
        assert!(source.matches_uri("rtsp://192.168.1.10/Profile1"));
        assert!(!source.matches_uri("rtsp://192.168.1.10/Profile2"));
        assert!(!source.matches_uri("rtsp://192.168.1.11/Profile1"));
    }

    #[test]
    fn onvif_settings_keep_identity_without_cached_address_and_read_legacy_sources() {
        let mut source = VideoSourceOnvif::explicit_rtsp("rtsp://192.168.1.10/Profile1").unwrap();
        source.source = VideoSourceOnvifType::Onvif(
            "onvif://bc071801-c50f-8301-ac36-bc071801c50f/Profile1".into(),
        );
        let saved = serde_json::to_string(&source).unwrap();
        assert!(!saved.contains("\"stream_uri\""));
        let restored: VideoSourceOnvif = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored.source, source.source);
        assert!(restored.stream_uri.is_none());

        let legacy = r#"{"name":"Camera","source":{"Onvif":"rtsp://192.168.1.10/Profile1"},"manufacturer":"Test","model":"Test","firmware_version":"1","serial_number":"123","hardware_id":"Test"}"#;
        let restored: VideoSourceOnvif = serde_json::from_str(legacy).unwrap();
        assert_eq!(restored.source_string(), "rtsp://192.168.1.10/Profile1");
        assert!(restored.stream_uri.is_none());
    }
}
