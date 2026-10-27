use std::sync::atomic::{AtomicUsize, Ordering};

use once_cell::sync::Lazy;

use teloxide::Bot;

use crate::config;

pub struct RoundRobinBot {
    bots: Vec<Bot>,
    current_index: AtomicUsize,
}

impl RoundRobinBot {
    pub fn new(bot_tokens: Vec<String>) -> Self {
        assert!(!bot_tokens.is_empty(), "BOT_TOKENS must not be empty");
        RoundRobinBot {
            bots: bot_tokens.into_iter().map(Bot::new).collect(),
            current_index: AtomicUsize::new(0),
        }
    }

    pub fn get_bot(&self) -> Bot {
        let index = self.current_index.fetch_add(1, Ordering::Relaxed) % self.bots.len();
        self.bots[index].clone()
    }
}

pub static ROUND_ROBIN_BOT: Lazy<RoundRobinBot> =
    Lazy::new(|| RoundRobinBot::new(config::CONFIG.bot_tokens.clone()));
