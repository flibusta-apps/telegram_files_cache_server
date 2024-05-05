use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use once_cell::sync::Lazy;

use teloxide::Bot;

use crate::config;

pub struct RoundRobinBot {
    bot_tokens: Arc<Vec<String>>,
    current_index: AtomicUsize,
}

impl RoundRobinBot {
    pub fn new(bot_tokens: Vec<String>) -> Self {
        RoundRobinBot {
            bot_tokens: Arc::new(bot_tokens),
            current_index: AtomicUsize::new(0),
        }
    }

    pub fn get_bot(&self) -> Bot {
        let index = self.current_index.fetch_add(1, Ordering::Relaxed) % self.bot_tokens.len();
        Bot::new(self.bot_tokens[index].clone())
    }
}

pub static ROUND_ROBIN_BOT: Lazy<RoundRobinBot> =
    Lazy::new(|| RoundRobinBot::new(config::CONFIG.bot_tokens.clone()));
