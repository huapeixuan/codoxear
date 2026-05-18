use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryRequest {
    pub base_url: String,
    pub model: String,
    pub api_key_present: bool,
    pub target_words: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtsRequest {
    pub base_url: String,
    pub model: String,
    pub voice: String,
    pub api_key_present: bool,
    pub response_format: &'static str,
}

pub trait OpenAiVoiceClient: Send + Sync {
    fn summarize(&self, request: SummaryRequest, text: &str) -> Result<String, String>;
    fn synthesize(&self, request: TtsRequest, text: &str) -> Result<Vec<u8>, String>;
}

#[derive(Debug, Clone)]
pub struct DisabledOpenAiClient;

impl OpenAiVoiceClient for DisabledOpenAiClient {
    fn summarize(&self, _request: SummaryRequest, _text: &str) -> Result<String, String> {
        Err("OpenAI client not configured".to_string())
    }

    fn synthesize(&self, _request: TtsRequest, _text: &str) -> Result<Vec<u8>, String> {
        Err("OpenAI client not configured".to_string())
    }
}

pub fn default_timeout() -> Duration {
    Duration::from_secs(30)
}
