/// Configuration options for a test container.
///
/// Pass this to any container helper to override the default Docker image tag
/// or the command-line arguments passed to the container process.
///
/// # Example
///
/// ```rust,ignore
/// use test_containers_util::Options;
///
/// let opts = Options {
///     tag: "16-alpine".to_string(),
///     cmd: vec!["-c".to_string(), "max_connections=200".to_string()],
/// };
/// ```
pub struct Options {
    /// Docker image tag to use (e.g. `"18-alpine"`, `"latest"`).
    pub tag: String,
    /// Arguments appended to the container's entrypoint command.
    pub cmd: Vec<String>,
}
