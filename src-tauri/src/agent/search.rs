use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub published_at: Option<String>,
    pub snippet: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchRequest {
    pub query: String,
    pub limit: u8,
}

pub trait WebSearchProvider: Send + Sync {
    fn search(&self, request: &SearchRequest) -> Result<Vec<SearchResult>, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MockSearchProvider;

impl WebSearchProvider for MockSearchProvider {
    fn search(&self, request: &SearchRequest) -> Result<Vec<SearchResult>, String> {
        let query = request.query.trim();
        if query.is_empty() {
            return Err("query must not be empty".into());
        }
        let limit = usize::from(request.limit.clamp(1, 10));
        Ok((0..limit)
            .map(|index| SearchResult {
                title: format!("Mock result {index}: {query}"),
                url: format!("https://search.example.test/result/{index}"),
                published_at: None,
                snippet: format!("Offline fixture for query: {query}"),
                source: "mock".into(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_result_keeps_traceable_source_fields() {
        let results = MockSearchProvider
            .search(&SearchRequest {
                query: "rust".into(),
                limit: 2,
            })
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].source, "mock");
        assert!(results[0].url.starts_with("https://"));
    }

    #[test]
    fn empty_search_is_rejected_and_limit_is_bounded() {
        assert!(MockSearchProvider
            .search(&SearchRequest {
                query: " ".into(),
                limit: 2
            })
            .is_err());
        let results = MockSearchProvider
            .search(&SearchRequest {
                query: "bounded".into(),
                limit: 255,
            })
            .unwrap();
        assert_eq!(results.len(), 10);
    }
}
