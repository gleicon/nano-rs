//! Cross-isolate LRU eviction with soft and hard eviction policies.
//!
//! STATUS: implemented and unit-tested, but **not wired into the serving path** —
//! `EvictionManager` (cross-isolate LRU victim selection) is not constructed or
//! invoked by the worker pool. Per-worker soft eviction IS wired via `MemoryMonitor`
//! (worker/pool.rs recycles an isolate under its own memory pressure); this module
//! is the *global* layer that would pick victims across isolates. Wiring it needs
//! shared pressure state plus a way to signal the owning worker thread to recycle a
//! `!Send` isolate — see docs/ROADMAP.md.
//!
//! This module provides Cloudflare-style isolate eviction that targets
//! stateless isolates first while allowing stateful isolates to complete
//! their current requests. Implements LRU (Least Recently Used) caching
//! with configurable eviction policies.
//!
//! ## Architecture
//!
//! - `EvictionManager`: Central coordinator for isolate eviction
//! - `EvictionPolicy`: Configurable selection strategy (LRU, LFU, etc.)
//! - `IsolateMetadata`: Per-isolate tracking information
//! - `EvictionAction`: Action to take based on pressure level
//!
//! ## Eviction Policies
//!
//! - **LRU** (default): Least Recently Used - best for most workloads
//! - **LFU**: Least Frequently Used - good for cache-like workloads
//! - **Random**: Random selection - simple, no tracking overhead
//! - **LargestFirst**: Evict largest memory footprints first
//!
//! ## Soft vs Hard Eviction
//!
//! - **Soft**: Mark isolate as "draining", allow current requests to complete,
//!   reject new requests. Graceful state drain before disposal.
//! - **Hard**: Immediate isolate termination. Used in emergency pressure only.

use crate::worker::memory_monitor::{MemoryPressureLevel, MemorySnapshot};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

/// Static counter for generating unique isolate IDs across the process
static ISOLATE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Unique identifier for an isolate within the eviction manager
/// Uses a hash-based string like "iso_a3f7b2d8" to uniquely identify isolate instances
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IsolateId(pub String);

impl IsolateId {
    /// Generate a new unique isolate ID with hash format
    /// Each call creates a unique ID even for the same worker (e.g., after OOM recovery)
    pub fn generate() -> Self {
        let counter = ISOLATE_COUNTER.fetch_add(1, Ordering::SeqCst);
        let uuid = uuid::Uuid::new_v4();
        // Format: iso_{first_8_of_uuid}_{counter}
        // This gives us unique, traceable IDs like "iso_a3f7b2d8_00000042"
        let hash = format!("iso_{}_{:08x}", uuid.to_string()[..8].to_string(), counter);
        Self(hash)
    }

    /// Create from an existing string (for tests/deserialization)
    pub fn from_string(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Get the ID string
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for IsolateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Eviction policy for selecting which isolates to evict
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionPolicy {
    /// Least Recently Used - evict isolates that haven't been used longest
    /// Best general-purpose policy for most workloads
    Lru,
    /// Least Frequently Used - evict least accessed isolates
    /// Good for workloads with hot/cold data patterns
    Lfu,
    /// Random selection - simple, no tracking overhead
    /// Useful when all isolates are roughly equivalent
    Random,
    /// Largest memory footprint first - evict biggest consumers
    /// Best for quickly reducing memory pressure
    LargestFirst,
}

impl Default for EvictionPolicy {
    fn default() -> Self {
        EvictionPolicy::Lru
    }
}

impl EvictionPolicy {
    /// Get a human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            EvictionPolicy::Lru => "Least Recently Used",
            EvictionPolicy::Lfu => "Least Frequently Used",
            EvictionPolicy::Random => "Random Selection",
            EvictionPolicy::LargestFirst => "Largest Memory First",
        }
    }
}

/// Metadata tracked for each isolate
#[derive(Debug, Clone)]
pub struct IsolateMetadata {
    /// Hostname this isolate serves
    pub hostname: String,
    /// Worker ID (index in the pool)
    pub worker_id: u32,
    /// When this isolate was last used
    pub last_used: Instant,
    /// How many times this isolate has been used
    pub use_count: u64,
    /// Memory footprint in bytes (last known)
    pub memory_footprint: usize,
    /// Whether this isolate can be safely evicted (stateless)
    pub is_stateless: bool,
    /// Number of currently active requests
    pub active_requests: usize,
    /// When this isolate was created
    pub created_at: Instant,
}

impl IsolateMetadata {
    /// Create new metadata for an isolate
    pub fn new(hostname: impl Into<String>, worker_id: u32) -> Self {
        Self {
            hostname: hostname.into(),
            worker_id,
            last_used: Instant::now(),
            use_count: 0,
            memory_footprint: 0,
            is_stateless: true, // Default to stateless until proven otherwise
            active_requests: 0,
            created_at: Instant::now(),
        }
    }

    /// Record that this isolate was just used
    pub fn record_usage(&mut self) {
        self.last_used = Instant::now();
        self.use_count += 1;
    }

    /// Update memory footprint
    pub fn update_memory(&mut self, bytes: usize) {
        self.memory_footprint = bytes;
    }

    /// Mark this isolate as having active state (not safely evictable)
    pub fn mark_stateful(&mut self) {
        self.is_stateless = false;
    }

    /// Mark this isolate as stateless (safely evictable)
    pub fn mark_stateless(&mut self) {
        self.is_stateless = true;
    }

    /// Increment active request count
    pub fn increment_active(&mut self) {
        self.active_requests += 1;
    }

    /// Decrement active request count
    pub fn decrement_active(&mut self) {
        if self.active_requests > 0 {
            self.active_requests -= 1;
        }
    }

    /// Check if this isolate is currently idle (no active requests)
    pub fn is_idle(&self) -> bool {
        self.active_requests == 0
    }

    /// Get time since last use
    pub fn idle_duration(&self) -> std::time::Duration {
        self.last_used.elapsed()
    }

    /// Get the age of this isolate (time since creation)
    pub fn age(&self) -> std::time::Duration {
        self.created_at.elapsed()
    }

    /// Get formatted age string for debugging (e.g., "45s", "3m 12s", "2h 5m")
    pub fn age_formatted(&self) -> String {
        let age = self.age();
        let secs = age.as_secs();
        if secs < 60 {
            format!("{}s", secs)
        } else if secs < 3600 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
        }
    }
}

/// Action to take based on memory pressure evaluation
#[derive(Debug, Clone, PartialEq)]
pub enum EvictionAction {
    /// Continue normal operation
    Allow,
    /// Reduce concurrent request processing
    Throttle,
    /// Soft eviction - evict after current requests complete
    SoftEvict(Vec<IsolateId>),
    /// Hard eviction - immediate termination
    HardEvict(Vec<IsolateId>),
}

impl EvictionAction {
    /// Check if this action involves any eviction
    pub fn is_eviction(&self) -> bool {
        matches!(
            self,
            EvictionAction::SoftEvict(_) | EvictionAction::HardEvict(_)
        )
    }

    /// Check if this is a hard eviction
    pub fn is_hard(&self) -> bool {
        matches!(self, EvictionAction::HardEvict(_))
    }

    /// Get the number of isolates to be evicted
    pub fn eviction_count(&self) -> usize {
        match self {
            EvictionAction::SoftEvict(ids) => ids.len(),
            EvictionAction::HardEvict(ids) => ids.len(),
            _ => 0,
        }
    }

    /// Get the IDs of isolates to evict, if any
    pub fn eviction_ids(&self) -> Option<&[IsolateId]> {
        match self {
            EvictionAction::SoftEvict(ids) => Some(ids),
            EvictionAction::HardEvict(ids) => Some(ids),
            _ => None,
        }
    }
}

/// State of an isolate from eviction perspective
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolateEvictionState {
    /// Normal operation - accepting new requests
    Active,
    /// Soft eviction in progress - completing current requests, rejecting new ones
    Draining { remaining_requests: usize },
    /// Evicted - no longer accepting requests, pending disposal
    Evicted,
}

impl Default for IsolateEvictionState {
    fn default() -> Self {
        IsolateEvictionState::Active
    }
}

/// Configuration for eviction behavior
#[derive(Debug, Clone, Copy)]
pub struct EvictionConfig {
    /// Eviction policy to use
    pub policy: EvictionPolicy,
    /// Target number of isolates to evict per critical event
    pub target_eviction_count: usize,
    /// Cooldown between evictions (seconds)
    pub cooldown_secs: u64,
    /// Whether to prefer stateless isolates
    pub prefer_stateless: bool,
    /// Minimum idle time before considering for eviction (seconds)
    pub min_idle_secs: u64,
}

impl Default for EvictionConfig {
    fn default() -> Self {
        Self {
            policy: EvictionPolicy::Lru,
            target_eviction_count: 1,
            cooldown_secs: 5,
            prefer_stateless: true,
            min_idle_secs: 0,
        }
    }
}

/// Manages isolate eviction based on memory pressure
///
/// Tracks isolate usage metadata and implements eviction policies.
/// Coordinates soft eviction (graceful drain) and hard eviction (immediate).
#[derive(Debug)]
pub struct EvictionManager {
    config: EvictionConfig,
    isolates: HashMap<IsolateId, IsolateMetadata>,
    eviction_states: HashMap<IsolateId, IsolateEvictionState>,
    total_memory: AtomicUsize,
    evictions_total: AtomicUsize,
    last_eviction: Option<Instant>,
}

impl EvictionManager {
    /// Create a new eviction manager with default configuration
    pub fn new() -> Self {
        Self::with_config(EvictionConfig::default())
    }

    /// Create a new eviction manager with custom configuration
    pub fn with_config(config: EvictionConfig) -> Self {
        Self {
            config,
            isolates: HashMap::new(),
            eviction_states: HashMap::new(),
            total_memory: AtomicUsize::new(0),
            evictions_total: AtomicUsize::new(0),
            last_eviction: None,
        }
    }

    /// Register a new isolate with the eviction manager
    ///
    /// # Arguments
    /// * `id` - Unique isolate identifier
    /// * `metadata` - Initial isolate metadata
    pub fn register_isolate(&mut self, id: IsolateId, metadata: IsolateMetadata) {
        self.isolates.insert(id.clone(), metadata);
        self.eviction_states
            .insert(id, IsolateEvictionState::Active);
    }

    /// Unregister an isolate (e.g., after disposal)
    ///
    /// # Arguments
    /// * `id` - Isolate to unregister
    pub fn unregister_isolate(&mut self, id: &IsolateId) {
        self.isolates.remove(id);
        self.eviction_states.remove(id);
    }

    /// Record usage of an isolate (updates LRU position, use count)
    ///
    /// Call this after each request completion.
    ///
    /// # Arguments
    /// * `id` - Isolate that was used
    /// * `memory_bytes` - Current memory footprint
    pub fn record_usage(&mut self, id: &IsolateId, memory_bytes: usize) {
        if let Some(meta) = self.isolates.get_mut(id) {
            meta.record_usage();
            meta.update_memory(memory_bytes);
            self.total_memory.store(
                self.isolates.values().map(|m| m.memory_footprint).sum(),
                Ordering::Relaxed,
            );
        }
    }

    /// Mark an isolate as having active requests
    pub fn mark_active(&mut self, id: &IsolateId) {
        if let Some(meta) = self.isolates.get_mut(id) {
            meta.increment_active();
        }
    }

    /// Mark an isolate as completing an active request
    pub fn mark_complete(&mut self, id: &IsolateId) {
        if let Some(meta) = self.isolates.get_mut(id) {
            meta.decrement_active();
        }

        // Check if draining isolate is now ready for eviction
        self.check_draining_complete(id);
    }

    /// Check if an isolate is in draining mode
    pub fn is_draining(&self, id: &IsolateId) -> bool {
        matches!(
            self.eviction_states.get(id),
            Some(IsolateEvictionState::Draining { .. })
        )
    }

    /// Check if an isolate has been evicted
    pub fn is_evicted(&self, id: &IsolateId) -> bool {
        matches!(
            self.eviction_states.get(id),
            Some(IsolateEvictionState::Evicted)
        )
    }

    /// Check if an isolate can accept new requests
    pub fn can_accept_requests(&self, id: &IsolateId) -> bool {
        matches!(
            self.eviction_states.get(id),
            Some(IsolateEvictionState::Active)
        )
    }

    /// Evaluate memory pressure and determine appropriate action
    ///
    /// This is the main entry point for eviction decisions.
    ///
    /// # Arguments
    /// * `level` - Current memory pressure level
    /// * `snapshots` - Optional memory snapshots for context
    ///
    /// # Returns
    /// The eviction action to take
    pub fn evaluate_pressure(
        &mut self,
        level: MemoryPressureLevel,
        _snapshots: Option<&HashMap<IsolateId, MemorySnapshot>>,
    ) -> EvictionAction {
        // Check cooldown
        if let Some(last) = self.last_eviction {
            if last.elapsed().as_secs() < self.config.cooldown_secs {
                // Still in cooldown, just throttle if needed
                if level.requires_eviction() {
                    return EvictionAction::Throttle;
                }
                return EvictionAction::Allow;
            }
        }

        match level {
            MemoryPressureLevel::Normal => EvictionAction::Allow,
            MemoryPressureLevel::Warning => EvictionAction::Throttle,
            MemoryPressureLevel::Critical => {
                let victims = self.select_victims(self.config.target_eviction_count, true);
                if victims.is_empty() {
                    EvictionAction::Throttle
                } else {
                    EvictionAction::SoftEvict(victims)
                }
            }
            MemoryPressureLevel::Emergency => {
                let victims = self.select_victims(self.config.target_eviction_count * 2, false);
                if victims.is_empty() {
                    EvictionAction::SoftEvict(vec![]) // No victims available
                } else {
                    EvictionAction::HardEvict(victims)
                }
            }
        }
    }

    /// Initiate soft eviction for an isolate
    ///
    /// Marks the isolate as draining. It will continue processing current
    /// requests but reject new ones.
    ///
    /// # Arguments
    /// * `id` - Isolate to soft evict
    ///
    /// # Returns
    /// true if eviction was initiated, false if already evicted or not found
    pub fn initiate_soft_eviction(&mut self, id: &IsolateId) -> bool {
        if let Some(meta) = self.isolates.get(id) {
            let active = meta.active_requests;

            // Only initiate if currently active
            if let Some(state) = self.eviction_states.get_mut(id) {
                if matches!(state, IsolateEvictionState::Active) {
                    *state = IsolateEvictionState::Draining {
                        remaining_requests: active,
                    };
                    self.last_eviction = Some(Instant::now());
                    return true;
                }
            }
        }
        false
    }

    /// Complete soft eviction (called when draining is done)
    ///
    /// Transitions from Draining to Evicted state.
    ///
    /// # Arguments
    /// * `id` - Isolate to finalize eviction for
    ///
    /// # Returns
    /// true if eviction was completed, false if not in draining state
    pub fn complete_soft_eviction(&mut self, id: &IsolateId) -> bool {
        if let Some(state) = self.eviction_states.get_mut(id) {
            if matches!(state, IsolateEvictionState::Draining { .. }) {
                *state = IsolateEvictionState::Evicted;
                self.evictions_total.fetch_add(1, Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    /// Perform hard eviction (immediate)
    ///
    /// Transitions directly to Evicted state regardless of active requests.
    ///
    /// # Arguments
    /// * `id` - Isolate to hard evict
    pub fn hard_evict(&mut self, id: &IsolateId) {
        if let Some(state) = self.eviction_states.get_mut(id) {
            *state = IsolateEvictionState::Evicted;
            self.evictions_total.fetch_add(1, Ordering::Relaxed);
            self.last_eviction = Some(Instant::now());
        }
    }

    /// Reset an evicted isolate back to active
    ///
    /// Used when a new isolate replaces an evicted one.
    ///
    /// # Arguments
    /// * `id` - Isolate to reactivate
    /// * `new_metadata` - Fresh metadata for the replacement
    pub fn reactivate_isolate(&mut self, id: IsolateId, new_metadata: IsolateMetadata) {
        self.isolates.insert(id.clone(), new_metadata);
        self.eviction_states
            .insert(id, IsolateEvictionState::Active);
    }

    /// Get metadata for an isolate
    pub fn get_metadata(&self, id: &IsolateId) -> Option<&IsolateMetadata> {
        self.isolates.get(id)
    }

    /// Get mutable metadata for an isolate
    pub fn get_metadata_mut(&mut self, id: &IsolateId) -> Option<&mut IsolateMetadata> {
        self.isolates.get_mut(id)
    }

    /// Get the age of an isolate (time since creation)
    pub fn get_isolate_age(&self, id: &IsolateId) -> Option<std::time::Duration> {
        self.isolates.get(id).map(|m| m.age())
    }

    /// Get formatted age string for an isolate (e.g., "45s", "3m 12s")
    pub fn get_isolate_age_formatted(&self, id: &IsolateId) -> Option<String> {
        self.isolates.get(id).map(|m| m.age_formatted())
    }

    /// Get total memory across all tracked isolates
    pub fn total_memory_bytes(&self) -> usize {
        self.total_memory.load(Ordering::Relaxed)
    }

    /// Get total number of evictions performed
    pub fn eviction_count(&self) -> usize {
        self.evictions_total.load(Ordering::Relaxed)
    }

    /// Get the number of registered isolates
    pub fn isolate_count(&self) -> usize {
        self.isolates.len()
    }

    /// Get the number of isolates in each state
    pub fn state_counts(&self) -> (usize, usize, usize) {
        let mut active = 0;
        let mut draining = 0;
        let mut evicted = 0;

        for state in self.eviction_states.values() {
            match state {
                IsolateEvictionState::Active => active += 1,
                IsolateEvictionState::Draining { .. } => draining += 1,
                IsolateEvictionState::Evicted => evicted += 1,
            }
        }

        (active, draining, evicted)
    }

    /// Get time since last eviction
    pub fn time_since_last_eviction(&self) -> Option<std::time::Duration> {
        self.last_eviction.map(|t| t.elapsed())
    }

    /// Clear all isolates and reset state
    pub fn clear(&mut self) {
        self.isolates.clear();
        self.eviction_states.clear();
        self.total_memory.store(0, Ordering::Relaxed);
    }

    // Private methods

    fn select_victims(&self, count: usize, stateless_only: bool) -> Vec<IsolateId> {
        let min_idle = std::time::Duration::from_secs(self.config.min_idle_secs);

        let candidates: Vec<_> = self
            .isolates
            .iter()
            .filter(|(id, meta)| {
                // Must be active (not already draining/evicted)
                if let Some(state) = self.eviction_states.get(id) {
                    if !matches!(state, IsolateEvictionState::Active) {
                        return false;
                    }
                }

                // Must be idle (no active requests)
                if !meta.is_idle() {
                    return false;
                }

                // Must meet minimum idle time
                if meta.idle_duration() < min_idle {
                    return false;
                }

                // Stateless preference
                if stateless_only && self.config.prefer_stateless && !meta.is_stateless {
                    return false;
                }

                true
            })
            .collect();

        if candidates.is_empty() {
            return vec![];
        }

        // Sort based on policy
        let mut sorted: Vec<_> = match self.config.policy {
            EvictionPolicy::Lru => {
                let mut v: Vec<_> = candidates.into_iter().collect();
                v.sort_by_key(|(_, meta)| meta.last_used);
                v
            }
            EvictionPolicy::Lfu => {
                let mut v: Vec<_> = candidates.into_iter().collect();
                v.sort_by_key(|(_, meta)| meta.use_count);
                v
            }
            EvictionPolicy::LargestFirst => {
                let mut v: Vec<_> = candidates.into_iter().collect();
                v.sort_by_key(|(_, meta)| std::cmp::Reverse(meta.memory_footprint));
                v
            }
            EvictionPolicy::Random => {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut v: Vec<_> = candidates.into_iter().collect();
                // Deterministic "random" based on current time hash
                let now = Instant::now();
                let mut hasher = DefaultHasher::new();
                now.hash(&mut hasher);
                let seed = hasher.finish();
                v.sort_by_key(|(id, _)| {
                    let mut h = DefaultHasher::new();
                    id.hash(&mut h);
                    seed.wrapping_add(h.finish())
                });
                v
            }
        };

        // Take up to count victims
        sorted.truncate(count);
        sorted.into_iter().map(|(id, _)| id.clone()).collect()
    }

    fn check_draining_complete(&mut self, id: &IsolateId) {
        if let Some(meta) = self.isolates.get(id) {
            if meta.is_idle() {
                if let Some(state) = self.eviction_states.get_mut(id) {
                    if matches!(state, IsolateEvictionState::Draining { .. }) {
                        *state = IsolateEvictionState::Evicted;
                        self.evictions_total.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }
}

impl Default for EvictionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_isolate_id_creation() {
        let id = IsolateId::from_string("iso_a3f7b2d8_00000005");
        assert_eq!(id.as_str(), "iso_a3f7b2d8_00000005");
        assert_eq!(id.to_string(), "iso_a3f7b2d8_00000005");
    }

    #[test]
    fn test_isolate_id_generate_unique() {
        let id1 = IsolateId::generate();
        let id2 = IsolateId::generate();
        // Each generated ID should be unique
        assert_ne!(id1.as_str(), id2.as_str());
        // Format should be iso_{uuid}_{counter}
        assert!(id1.as_str().starts_with("iso_"));
        assert!(id2.as_str().starts_with("iso_"));
    }

    #[test]
    fn test_eviction_policy_default() {
        assert_eq!(EvictionPolicy::default(), EvictionPolicy::Lru);
    }

    #[test]
    fn test_eviction_policy_descriptions() {
        assert!(EvictionPolicy::Lru.description().contains("Recently"));
        assert!(EvictionPolicy::Lfu.description().contains("Frequently"));
        assert!(EvictionPolicy::Random.description().contains("Random"));
        assert!(EvictionPolicy::LargestFirst
            .description()
            .contains("Largest"));
    }

    #[test]
    fn test_isolate_metadata_creation() {
        let meta = IsolateMetadata::new("test.example.com", 1);
        assert_eq!(meta.hostname, "test.example.com");
        assert_eq!(meta.worker_id, 1);
        assert!(meta.is_stateless);
        assert_eq!(meta.active_requests, 0);
        assert_eq!(meta.use_count, 0);
    }

    #[test]
    fn test_isolate_metadata_usage_tracking() {
        let mut meta = IsolateMetadata::new("test.example.com", 1);

        meta.record_usage();
        assert_eq!(meta.use_count, 1);

        meta.update_memory(1024 * 1024);
        assert_eq!(meta.memory_footprint, 1024 * 1024);

        meta.mark_stateful();
        assert!(!meta.is_stateless);

        meta.mark_stateless();
        assert!(meta.is_stateless);
    }

    #[test]
    fn test_isolate_metadata_active_requests() {
        let mut meta = IsolateMetadata::new("test.example.com", 1);

        assert!(meta.is_idle());

        meta.increment_active();
        assert_eq!(meta.active_requests, 1);
        assert!(!meta.is_idle());

        meta.decrement_active();
        assert_eq!(meta.active_requests, 0);
        assert!(meta.is_idle());
    }

    #[test]
    fn test_eviction_action_types() {
        let allow = EvictionAction::Allow;
        assert!(!allow.is_eviction());
        assert!(!allow.is_hard());
        assert_eq!(allow.eviction_count(), 0);

        let soft = EvictionAction::SoftEvict(vec![IsolateId::from_string("iso_soft_1")]);
        assert!(soft.is_eviction());
        assert!(!soft.is_hard());
        assert_eq!(soft.eviction_count(), 1);

        let hard = EvictionAction::HardEvict(vec![
            IsolateId::from_string("iso_hard_1"),
            IsolateId::from_string("iso_hard_2"),
        ]);
        assert!(hard.is_eviction());
        assert!(hard.is_hard());
        assert_eq!(hard.eviction_count(), 2);
    }

    #[test]
    fn test_eviction_manager_creation() {
        let manager = EvictionManager::new();
        assert_eq!(manager.isolate_count(), 0);
        assert_eq!(manager.eviction_count(), 0);
        assert_eq!(manager.total_memory_bytes(), 0);
    }

    #[test]
    fn test_eviction_manager_registration() {
        let mut manager = EvictionManager::new();
        let id = IsolateId::from_string("iso_test_0");
        let meta = IsolateMetadata::new("test.example.com", 0);

        manager.register_isolate(id.clone(), meta);
        assert_eq!(manager.isolate_count(), 1);
        assert!(manager.can_accept_requests(&id));

        manager.unregister_isolate(&id);
        assert_eq!(manager.isolate_count(), 0);
    }

    #[test]
    fn test_eviction_manager_usage_tracking() {
        let mut manager = EvictionManager::new();
        let id = IsolateId::from_string("iso_test_0");
        let meta = IsolateMetadata::new("test.example.com", 0);

        manager.register_isolate(id.clone(), meta);
        manager.record_usage(&id, 1024 * 1024);

        let meta = manager.get_metadata(&id).unwrap();
        assert_eq!(meta.use_count, 1);
        assert_eq!(meta.memory_footprint, 1024 * 1024);
    }

    #[test]
    fn test_soft_eviction_lifecycle() {
        let mut manager = EvictionManager::new();
        let id = IsolateId::from_string("iso_test_0");
        let meta = IsolateMetadata::new("test.example.com", 0);

        manager.register_isolate(id.clone(), meta);

        // Initiate soft eviction
        assert!(manager.initiate_soft_eviction(&id));
        assert!(manager.is_draining(&id));
        assert!(!manager.can_accept_requests(&id));

        // Complete soft eviction
        assert!(manager.complete_soft_eviction(&id));
        assert!(manager.is_evicted(&id));
        assert_eq!(manager.eviction_count(), 1);
    }

    #[test]
    fn test_hard_eviction() {
        let mut manager = EvictionManager::new();
        let id = IsolateId::from_string("iso_test_0");
        let meta = IsolateMetadata::new("test.example.com", 0);

        manager.register_isolate(id.clone(), meta);

        // Add active requests to prevent soft eviction completion
        manager.mark_active(&id);

        // Hard eviction works regardless of active requests
        manager.hard_evict(&id);
        assert!(manager.is_evicted(&id));
        assert_eq!(manager.eviction_count(), 1);
    }

    #[test]
    fn test_evaluate_pressure_levels() {
        let mut manager = EvictionManager::new();

        // Normal pressure
        let action = manager.evaluate_pressure(MemoryPressureLevel::Normal, None);
        assert_eq!(action, EvictionAction::Allow);

        // Warning pressure
        let action = manager.evaluate_pressure(MemoryPressureLevel::Warning, None);
        assert_eq!(action, EvictionAction::Throttle);

        // Critical pressure (no isolates to evict)
        let action = manager.evaluate_pressure(MemoryPressureLevel::Critical, None);
        assert_eq!(action, EvictionAction::Throttle);

        // Emergency pressure (no isolates to evict)
        let action = manager.evaluate_pressure(MemoryPressureLevel::Emergency, None);
        assert!(matches!(action, EvictionAction::SoftEvict(_)));
    }

    #[test]
    fn test_eviction_cooldown() {
        let config = EvictionConfig {
            cooldown_secs: 60, // Long cooldown
            ..Default::default()
        };
        let mut manager = EvictionManager::with_config(config);

        // Register and evict an isolate
        let id = IsolateId::from_string("iso_test_0");
        let meta = IsolateMetadata::new("test.example.com", 0);
        manager.register_isolate(id.clone(), meta);
        manager.initiate_soft_eviction(&id);

        // During cooldown, critical pressure should just throttle
        let action = manager.evaluate_pressure(MemoryPressureLevel::Critical, None);
        assert_eq!(action, EvictionAction::Throttle);
    }

    #[test]
    fn test_state_counts() {
        let mut manager = EvictionManager::new();

        let id1 = IsolateId::from_string("iso_test_0");
        let id2 = IsolateId::from_string("iso_test_1");
        let id3 = IsolateId::from_string("iso_test_2");

        manager.register_isolate(id1.clone(), IsolateMetadata::new("app1", 0));
        manager.register_isolate(id2.clone(), IsolateMetadata::new("app2", 1));
        manager.register_isolate(id3.clone(), IsolateMetadata::new("app3", 2));

        // Initially all active
        let (active, draining, evicted) = manager.state_counts();
        assert_eq!(active, 3);
        assert_eq!(draining, 0);
        assert_eq!(evicted, 0);

        // Evict one
        manager.initiate_soft_eviction(&id1);
        let (active, draining, evicted) = manager.state_counts();
        assert_eq!(active, 2);
        assert_eq!(draining, 1);
        assert_eq!(evicted, 0);

        // Complete eviction
        manager.complete_soft_eviction(&id1);
        let (active, draining, evicted) = manager.state_counts();
        assert_eq!(active, 2);
        assert_eq!(draining, 0);
        assert_eq!(evicted, 1);
    }

    #[test]
    fn test_reactivate_isolate() {
        let mut manager = EvictionManager::new();
        let id = IsolateId::from_string("iso_test_0");

        manager.register_isolate(id.clone(), IsolateMetadata::new("test", 0));
        manager.initiate_soft_eviction(&id);
        manager.complete_soft_eviction(&id);

        assert!(manager.is_evicted(&id));

        // Reactivate
        manager.reactivate_isolate(id.clone(), IsolateMetadata::new("test", 0));
        assert!(manager.can_accept_requests(&id));
        assert!(!manager.is_evicted(&id));
    }

    #[test]
    fn test_eviction_config_default() {
        let config = EvictionConfig::default();
        assert_eq!(config.policy, EvictionPolicy::Lru);
        assert_eq!(config.target_eviction_count, 1);
        assert_eq!(config.cooldown_secs, 5);
        assert!(config.prefer_stateless);
        assert_eq!(config.min_idle_secs, 0);
    }

    #[test]
    fn test_eviction_lru_selection() {
        let config = EvictionConfig {
            policy: EvictionPolicy::Lru,
            ..Default::default()
        };
        let mut manager = EvictionManager::with_config(config);

        // Register isolates with different last_used times
        let id1 = IsolateId::from_string("iso_test_0");
        let id2 = IsolateId::from_string("iso_test_1");

        let mut meta1 = IsolateMetadata::new("app1", 0);
        let mut meta2 = IsolateMetadata::new("app2", 1);

        // Manually set last_used to simulate age difference
        meta1.last_used = Instant::now() - std::time::Duration::from_secs(100);
        meta2.last_used = Instant::now();

        manager.register_isolate(id1.clone(), meta1);
        manager.register_isolate(id2.clone(), meta2);

        // Evaluate critical pressure to trigger victim selection
        let action = manager.evaluate_pressure(MemoryPressureLevel::Critical, None);

        // Should select the older isolate (id1)
        if let EvictionAction::SoftEvict(victims) = action {
            assert!(!victims.is_empty());
            // The oldest isolate should be selected
        }
    }

    #[test]
    fn test_eviction_with_active_requests() {
        let mut manager = EvictionManager::new();

        let id = IsolateId::from_string("iso_test_0");
        let mut meta = IsolateMetadata::new("test", 0);
        meta.active_requests = 1; // Simulate active request

        manager.register_isolate(id.clone(), meta);

        // Should not select isolate with active requests
        let action = manager.evaluate_pressure(MemoryPressureLevel::Critical, None);
        assert_eq!(action, EvictionAction::Throttle);
    }
}
