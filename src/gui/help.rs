//! 帮助纯逻辑（G7-09）。

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpTopic {
    pub id: String,
    pub title: String,
    pub body: String,
}

impl HelpTopic {
    pub fn new(id: &str, title: &str, body: &str) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            body: body.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HelpIndex {
    pub topics: Vec<HelpTopic>,
}

impl HelpIndex {
    pub fn new() -> Self {
        Self { topics: Vec::new() }
    }

    pub fn add(&mut self, topic: HelpTopic) {
        self.topics.push(topic);
    }

    pub fn find(&self, id: &str) -> Option<&HelpTopic> {
        self.topics.iter().find(|t| t.id == id)
    }

    pub fn search(&self, keyword: &str) -> Vec<&HelpTopic> {
        let kw = keyword.to_lowercase();
        self.topics
            .iter()
            .filter(|t| t.title.to_lowercase().contains(&kw) || t.body.to_lowercase().contains(&kw))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_search() {
        let mut idx = HelpIndex::new();
        idx.add(HelpTopic::new("k1", "快捷键", "Ctrl+O 打开"));
        assert_eq!(idx.search("打开").len(), 1);
        assert!(idx.find("k1").is_some());
    }
}
