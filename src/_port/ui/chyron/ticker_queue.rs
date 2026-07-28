use std::collections::{HashMap, VecDeque};

use log::{debug, trace};
use news_flash::models::{ArticleFilter, CategoryID, FeedID, Read as NfRead};
use ratatui::style::Color;

use crate::prelude::*;
use super::ticker::TickerItem;

/// Category metadata for round-robin cycling.
pub struct CategoryInfo {
    pub name: String,
    pub id: CategoryID,
    pub color: Color,
    pub unread_count: i64,
    pub latest_headline: Option<String>,
}

/// Default color palette for categories without explicit color mapping.
const DEFAULT_COLORS: &[Color] = &[
    Color::Green,
    Color::Blue,
    Color::Red,
    Color::Cyan,
    Color::Yellow,
    Color::Magenta,
    Color::LightGreen,
    Color::LightBlue,
    Color::LightRed,
    Color::LightCyan,
];

/// Resolve a color name string to a ratatui Color.
fn resolve_color(name: &str) -> Option<Color> {
    match name.to_lowercase().as_str() {
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "blue" => Some(Color::Blue),
        "cyan" => Some(Color::Cyan),
        "yellow" => Some(Color::Yellow),
        "magenta" => Some(Color::Magenta),
        "white" => Some(Color::White),
        "gray" | "grey" => Some(Color::Gray),
        "lightred" | "light_red" => Some(Color::LightRed),
        "lightgreen" | "light_green" => Some(Color::LightGreen),
        "lightblue" | "light_blue" => Some(Color::LightBlue),
        "lightcyan" | "light_cyan" => Some(Color::LightCyan),
        "lightyellow" | "light_yellow" => Some(Color::LightYellow),
        "lightmagenta" | "light_magenta" => Some(Color::LightMagenta),
        _ => None,
    }
}

/// Build the list of categories with their unread counts and assigned colors.
///
/// Pattern follows `src/ui/feeds_list/model.rs:80,92,206-211`:
/// 1. `get_feeds()` returns `(Vec<Feed>, Vec<FeedMapping>)` — FeedMapping has category_id + feed_id
/// 2. `get_categories()` returns `(Vec<Category>, Vec<CategoryMapping>)` (sync)
/// 3. `unread_count_feed_map(true)?` returns `HashMap<FeedID, i64>` (sync)
/// 4. Aggregate feed-level counts into category-level counts via FeedMapping
pub async fn build_category_list(
    news_flash_utils: &NewsFlashUtils,
    config: &Config,
) -> Vec<CategoryInfo> {
    let news_flash = news_flash_utils.news_flash_lock.read().await;

    // Step 1: get feeds to obtain FeedMappings (feed_id -> category_id)
    let feed_mappings = match news_flash.get_feeds() {
        Ok((_feeds, mappings)) => mappings,
        Err(e) => {
            debug!("Failed to get feeds: {}", e);
            return Vec::new();
        }
    };

    // Step 2: get categories (sync call)
    let (categories, _category_mappings) = match news_flash.get_categories() {
        Ok(result) => result,
        Err(e) => {
            debug!("Failed to get categories: {}", e);
            return Vec::new();
        }
    };

    // Step 3: get per-feed unread counts (sync call)
    let feed_unread_map: HashMap<FeedID, i64> = match news_flash.unread_count_feed_map(true) {
        Ok(map) => map,
        Err(e) => {
            debug!("Failed to get unread counts: {}", e);
            HashMap::new()
        }
    };

    // Step 4: build category-to-feeds mapping from FeedMapping
    let mut category_feed_map: HashMap<CategoryID, Vec<FeedID>> = HashMap::new();
    for mapping in &feed_mappings {
        category_feed_map
            .entry(mapping.category_id.clone())
            .or_default()
            .push(mapping.feed_id.clone());
    }

    // Step 5: aggregate unread counts per category and build result
    let mut result = Vec::new();
    for (idx, category) in categories.iter().enumerate() {
        let unread: i64 = category_feed_map
            .get(&category.category_id)
            .map(|feeds| {
                feeds
                    .iter()
                    .map(|fid| feed_unread_map.get(fid).copied().unwrap_or(0))
                    .sum()
            })
            .unwrap_or(0);

        let color = config
            .chyron
            .category_colors
            .get(&category.label)
            .and_then(|c| resolve_color(c))
            .unwrap_or(DEFAULT_COLORS[idx % DEFAULT_COLORS.len()]);

        // Get latest headline for this category
        let latest_headline = {
            let filter = ArticleFilter {
                categories: vec![category.category_id.clone()].into(),
                ..Default::default()
            };
            news_flash
                .get_articles(filter)
                .ok()
                .and_then(|articles| articles.first().and_then(|a| a.title.clone()))
        };

        result.push(CategoryInfo {
            name: category.label.clone(),
            id: category.category_id.clone(),
            color,
            unread_count: unread,
            latest_headline,
        });
    }

    // Sort by unread count descending so categories with most unread are cycled first
    result.sort_by(|a, b| b.unread_count.cmp(&a.unread_count));
    result
}

/// Fetch the next batch of unread headlines from the given category.
pub async fn fetch_category_headlines(
    news_flash_utils: &NewsFlashUtils,
    category: &CategoryInfo,
    limit: usize,
) -> Vec<TickerItem> {
    let news_flash = news_flash_utils.news_flash_lock.read().await;

    let filter = ArticleFilter {
        categories: vec![category.id.clone()].into(),
        unread: Some(NfRead::Unread),
        ..Default::default()
    };

    // get_articles takes filter by value, is sync (no .await)
    let articles = match news_flash.get_articles(filter) {
        Ok(articles) => articles,
        Err(e) => {
            debug!(
                "Failed to get articles for category {}: {}",
                category.name, e
            );
            return Vec::new();
        }
    };

    articles
        .into_iter()
        .take(limit)
        .map(|article| TickerItem {
            category: category.name.clone(),
            color: category.color,
            feed_name: String::new(),
            title: article.title.clone().unwrap_or_default(),
            url: article
                .url
                .as_ref()
                .map(|u| u.to_string())
                .unwrap_or_default(),
            article_id: Some(article.article_id.clone()),
            published: Some(article.date),
        })
        .collect()
}

/// Refill the ticker queue using round-robin category cycling.
/// Called when queue depth drops below `min_depth`.
pub async fn refill_queue(
    queue: &mut VecDeque<TickerItem>,
    categories: &[CategoryInfo],
    current_category_index: &mut usize,
    news_flash_utils: &NewsFlashUtils,
    min_depth: usize,
    batch_size: usize,
) {
    if queue.len() >= min_depth || categories.is_empty() {
        return;
    }

    let mut attempts = 0;
    let max_attempts = categories.len();

    while queue.len() < min_depth && attempts < max_attempts {
        let cat = &categories[*current_category_index % categories.len()];
        *current_category_index = (*current_category_index + 1) % categories.len();
        attempts += 1;

        if cat.unread_count == 0 {
            continue;
        }

        let items = fetch_category_headlines(news_flash_utils, cat, batch_size).await;
        trace!(
            "Fetched {} headlines from category {}",
            items.len(),
            cat.name
        );
        for item in items {
            queue.push_back(item);
        }
    }
}
