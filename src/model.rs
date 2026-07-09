use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Parallel {
    pub lang: String,
    pub text: String,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchGroup {
    pub zh_text: String,
    pub cbeta_id: Option<String>,
    pub title_zh: Option<String>,
    pub juan_num: Option<i64>,
    pub parallels: Vec<Parallel>,
}
