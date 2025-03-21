use chrono::Local;
use log::{set_boxed_logger, set_max_level, LevelFilter, Log, Metadata, Record, SetLoggerError};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::{Mutex, MutexGuard};
use std::thread;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogConfig {
    log_level: String,
}

impl LogConfig {
    /// Converts the log level string to a `LevelFilter` enum value
    ///
    /// # Arguments
    /// * `self` - Reference to the LogConfig instance
    ///
    /// # Returns
    /// `LevelFilter` matching the configured log level
    ///
    /// # Panics
    /// Panics if the log_level string contains an invalid value
    #[doc = r" Converts the log level string to a `LevelFilter` enum value"]
    pub fn level(&self) -> LevelFilter {
        match self.log_level.as_str() {
            "trace" => LevelFilter::Trace,
            "debug" => LevelFilter::Debug,
            "info" => LevelFilter::Info,
            "warn" => LevelFilter::Warn,
            "error" => LevelFilter::Error,
            _ => panic!("Invalid log level"),
        }
    }
}

impl Default for LogConfig {
    /// Creates a default LogConfig with log level set to "info"
    ///
    /// # Returns
    /// New `LogConfig` instance with default values
    fn default() -> Self {
        Self {
            log_level: String::from("info"),
        }
    }
}

pub struct HtmlLogger<W: Write + Send + 'static> {
    level: LevelFilter,
    writable: Mutex<W>,
}

impl<W: Write + Send + 'static> HtmlLogger<W> {
    /// Initializes the HTML logger as the global logger
    ///
    /// # Arguments
    /// * `log_level` - Maximum log level to capture
    /// * `writable` - Write implementation for HTML output
    ///
    /// # Returns
    /// `Result` indicating success or failure
    pub fn init(log_level: LevelFilter, writable: W) -> Result<(), SetLoggerError> {
        set_max_level(log_level);
        set_boxed_logger(HtmlLogger::new(log_level, writable))
    }

    /// Creates a new HtmlLogger instance with initialized HTML structure
    ///
    /// # Arguments
    /// * `log_level` - Maximum log level to capture
    /// * `writable` - Write implementation for HTML output
    ///
    /// # Returns
    /// Boxed HtmlLogger ready for use
    #[must_use]
    pub fn new(log_level: LevelFilter, writable: W) -> Box<HtmlLogger<W>> {
        let write_mutex = Mutex::new(writable);
        {
            let mut write_lock = write_mutex.lock().unwrap();
            let _ = write!(
                write_lock,
                "<!DOCTYPE html>
<html>
<head>
<meta charset='UTF-8'>
<meta name='viewport' content='width=device-width, initial-scale=1.0'>
<title>Log Viewer</title>\n"
            );
            Self::add_styles(&mut write_lock);
            Self::add_scripts(&mut write_lock);
            let _ = write!(
                write_lock,
                "\n</head>
<body>
<div class=\"log-page\">
<div class=\"filter-controls\">
<div id=\"targetFilter\"></div>
<button id=\"resetButton\">Show all</button>
</div>
<div class=\"log-container\">\n"
            );
        }

        Box::new(HtmlLogger {
            level: log_level,
            writable: write_mutex,
        })
    }

    /// Acquires a lock on the underlying writable resource
    ///
    /// # Returns
    /// Mutex guard providing access to the output writer
    pub fn get_writable(&self) -> MutexGuard<'_, W> {
        self.writable.lock().unwrap()
    }

    /// Writes CSS styles for the HTML logger interface
    ///
    /// # Arguments
    /// * `write_lock` - Mutable reference to the output writer
    fn add_styles(write_lock: &mut W) {
        write!(
            write_lock,
            "<style>
.log-page {{ display: flex; height: 100vh; padding: 20px; flex-direction: row-reverse; }}
.log-message {{ margin-left: 1em; display: flex; align-items: flex-start; font-size: 10pt }}
.logtext {{ flex-grow: 1; padding: 3px; border-radius: 2px }}
.error {{ background-color: #ffcccc; color: darkred; font-weight: bold }}
.warn {{ background-color: #ffff99; color: darkorange; font-weight: bold }}
.info {{ color: black; }}
.debug {{ color: #333333; }}
.trace {{ color: #666666; font-style: italic }}
.timestamp {{ font-weight: bold; padding-top: 3px; white-space: nowrap; }}

.filter-controls {{
    position: fixed;
    width: 250px;
    height: 400px;
    padding: 10px;
    background: #f5f5f5;
    display: flex;
    gap: 20px;
    align-items: center;
    flex-direction: column;
    margin-right: auto;
    top: auto;
    right: 20px;
    z-index: 100;
    border: #444444;
    box-shadow: 4px 4px 8px rgba(0, 0, 0, 0.3);
    border-style: solid;
    border-width: 2px;
    border-radius: 5px;
}}

.targetFilter {{
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
}}

.filter-checkbox {{
    width: 16px;
    height: 16px;
    cursor: pointer;
}}

.log-container {{
    width: calc(100% - 290px);
    margin-right: auto;
}}


#targetFilter {{
    height: 80%;
    padding: 5px;
    border: 1px solid #ccc;
    border-radius: 3px;
    background: white;
    display: flex;
    flex-direction: column;
    gap: 5px;
    overflow-y: auto;
    font-family: Helvetica New, sans-serif;
    font-size: 12px;
    width: calc(100% - 20px);
    margin: 10px;
}} 

#resetButton {{
    padding: 5px 20px;
    background: #e0e0e0;
    border: 1px solid #ccc;
    border-radius: 5px;
    cursor: pointer;
    font-family: Helvetica New, sans-serif;
}}

#resetButton:hover {{
    background: #d0d0d0;
}}

body {{ font-family: 'Courier New', Courier, monospace; }}
</style>"
        )
        .unwrap();
    }

    /// Writes JavaScript functionality for log filtering
    ///
    /// # Arguments
    /// * `write_lock` - Mutable reference to the output writer
    fn add_scripts(write_lock: &mut W) {
        write!(
            write_lock,
            "\n<script>\n
document.addEventListener('DOMContentLoaded', function() {{
    // Populate the dropdown menu
    var logEntries = document.querySelectorAll('.log-message');
    var targets = new Set();

    logEntries.forEach(function(entry) {{
        var logtextSpan  = entry.querySelector('.logtext');
        if (logtextSpan ) {{
            targets.add(logtextSpan .textContent.split('[')[2].split(']')[0]);
        }}
    }});

    // Create checkboxes for each unique target
    const checkboxContainers = [];

    var allCheckbox = document.createElement('input');
    allCheckbox.type = 'checkbox';
    allCheckbox.className = 'filter-checkbox';
    allCheckbox.value = 'All';
    allCheckbox.checked = true;
    const allLabel = document.createElement('label');
    allLabel.textContent = 'All';

    const checkboxLabelContainer = document.createElement('div');
    checkboxLabelContainer.className = 'checkbox-label-container';

    checkboxLabelContainer.appendChild(allCheckbox);
    checkboxLabelContainer.appendChild(allLabel);
    checkboxContainers.push(checkboxLabelContainer);

    // Add event listener to 'All' checkbox
    allCheckbox.addEventListener('change', function () {{
        const isChecked = allCheckbox.checked;
        checkboxContainers.slice(1).forEach(container => {{ // Skip the first checkbox (All)
            const checkbox = container.querySelector('.filter-checkbox');
            checkbox.checked = isChecked;
            checkbox.dispatchEvent(new Event('change'));
        }});
    }});
    
    targets.forEach(function(target) {{
        const checkbox = document.createElement('input');
        checkbox.type = 'checkbox';
        checkbox.className = 'filter-checkbox';
        checkbox.value = target;
        checkbox.checked = true;
        
        const label = document.createElement('label');
        label.textContent = `${{target}}`;
        label.htmlFor = `checkbox-${{target}}`;
        
        // Create a container for each checkbox and label
        const checkboxLabelContainer = document.createElement('div');
        checkboxLabelContainer.className = 'checkbox-label-container';

        checkboxLabelContainer.appendChild(checkbox);
        checkboxLabelContainer.appendChild(label);

        checkboxContainers.push(checkboxLabelContainer);
    }});

    checkboxContainers.slice(1).forEach(container => {{
        const checkbox = container.querySelector('.filter-checkbox');
        checkbox.addEventListener('change', function() {{
            var selectedTargets = checkboxContainers.slice(1).map(c => {{
                const thisCheckbox = c.querySelector('.filter-checkbox');
                return thisCheckbox.checked ? thisCheckbox.value : '';
            }}).filter(Boolean).join(', ');
            
            logEntries.forEach(function(entry) {{
                const logtextSpan = entry.querySelector('.logtext');
                if (logtextSpan && 
                    selectedTargets.includes(logtextSpan.textContent.split('[')[2].split(']')[0])) {{
                    // Reset to initial style
                    entry.style.display = ''; 
                }} else {{
                    entry.style.display = 'none';
                }}
            }});

            // Ensure the 'All' checkbox is unchecked if any individual checkbox is unchecked
            allCheckbox.checked = checkboxContainers.slice(1).every(c => {{
                const thisCheckbox = c.querySelector('.filter-checkbox');
                return thisCheckbox.checked;
            }});
        }});
    }});

    var selectElement = document.getElementById('targetFilter');
    checkboxContainers.forEach(box => {{
        selectElement.appendChild(box);
    }})

    // Event listener to reset button
    document.getElementById('resetButton').addEventListener('click', function() {{
        allCheckbox.checked = true;
        checkboxContainers.slice(1).forEach(container => {{ // Skip the first checkbox (All)
            const checkbox = container.querySelector('.filter-checkbox');
            checkbox.checked = true;
            checkbox.dispatchEvent(new Event('change')); // Manually trigger change event
        }});
    }});

}});
</script>"
        )
        .unwrap()
    }
}

impl<W: Write + Send + 'static> Log for HtmlLogger<W> {
    /// Determines if a log message should be processed based on its level
    ///
    /// # Arguments
    /// * `metadata` - Log metadata containing the level to check
    ///
    /// # Returns
    /// `true` if the message level is within the configured filter, `false` otherwise
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    /// Processes and writes a log record to HTML output
    ///
    /// # Arguments
    /// * `record` - Log record to process
    ///
    /// # Panics
    /// Panics if there's an error acquiring the write lock
    ///
    /// # Behavior
    /// - Skips processing if record level exceeds configured level
    /// - Formats timestamp, thread ID, and log target
    /// - Applies CSS classes based on log level
    /// - Writes HTML-structured log entry to output
    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        if record.target().is_empty() {
            return;
        }

        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S.%3f").to_string();
        let level = record.level().to_string().to_lowercase();

        // get thread id
        let id = format!("{:?}", thread::current().id());
        let id = id.replace("ThreadId(", "");
        let id = id.replace(")", "");

        let target = if record.target().starts_with("tibberator.") {
            record.target().to_string()
        } else {
            format!("extern.{}", record.target())
        };

        let message = format!(
    "<div class='log-message'><span class='timestamp'>[{}]</span> <span class='{} logtext'>[{}][{}] {}</span></div>\n",
    timestamp,
    level,
    id,
    target,
    record.args()
);

        let mut write_lock = self.writable.lock().unwrap();
        let _ = write!(write_lock, "{}", message);
    }

    /// Flushes buffered log entries and writes HTML closing tags
    ///
    /// # Panics
    /// Panics if there's an error acquiring the write lock or writing to the output
    fn flush(&self) {
        let mut write_lock = self.writable.lock().unwrap();
        let _ = write_lock.flush();
        let _ = write!(write_lock, "\n</div>\n</div>\n</body>\n</html>");
    }
}

impl<W: Write + Send + 'static> Drop for HtmlLogger<W> {
    /// Ensures proper cleanup by flushing logs when the logger is dropped
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::LevelFilter;
    use std::ops::Deref;

    #[test]
    fn test_html_logger_new_initializes_html() {
        let buffer: Vec<u8> = Vec::new();
        let logger = HtmlLogger::new(LevelFilter::Info, buffer);
        let writable = logger.get_writable();
        let content = String::from_utf8(writable.deref().clone()).expect("Content found");

        assert!(content.contains("<!DOCTYPE html>"));
        assert!(content.contains("<head>"));
        assert!(content.contains("<body>"));
        assert!(content.contains("<style>"));
    }

    #[test]
    fn test_log_config_default() {
        let config = LogConfig::default();
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn test_log_config_level() {
        let config = LogConfig {
            log_level: "debug".to_string(),
        };
        assert_eq!(config.level(), LevelFilter::Debug);

        let config = LogConfig {
            log_level: "trace".to_string(),
        };
        assert_eq!(config.level(), LevelFilter::Trace);

        let config = LogConfig {
            log_level: "info".to_string(),
        };
        assert_eq!(config.level(), LevelFilter::Info);

        let config = LogConfig {
            log_level: "warn".to_string(),
        };
        assert_eq!(config.level(), LevelFilter::Warn);

        let config = LogConfig {
            log_level: "error".to_string(),
        };
        assert_eq!(config.level(), LevelFilter::Error);
    }

    #[test]
    #[should_panic]
    fn test_log_config_level_invalid() {
        let config = LogConfig {
            log_level: "invalid".to_string(),
        };
        config.level();
    }

    #[test]
    fn test_html_logger_output_formatting() {
        let buffer: Vec<u8> = Vec::new();
        let logger = HtmlLogger::new(LevelFilter::Info, buffer);

        // Manually create log records to test formatting
        let info_record = log::Record::builder()
            .args(format_args!("[test] Info message"))
            .target("test")
            .level(log::Level::Info)
            .build();
        logger.log(&info_record);

        let error_record = log::Record::builder()
            .args(format_args!("[test] Error message"))
            .target("test")
            .level(log::Level::Error)
            .build();
        logger.log(&error_record);

        // Flush to write closing tags
        logger.flush();

        // Get logged content - need to access through the logger instance
        let content =
            String::from_utf8(logger.get_writable().clone()).expect("Valid UTF-8 content");

        // Verify HTML structure
        assert!(content.contains("<!DOCTYPE html>"));
        assert!(content.contains("<style>"));
        assert!(content.contains("</style>"));

        // Verify message formatting
        assert!(content.contains("<div class='log-message'>"));
        assert!(content.contains("<span class='timestamp'>"));

        // Verify CSS classes applied correctly
        assert!(content.contains("class='info logtext'"));
        assert!(content.contains("class='error logtext'"));

        // Verify message content
        assert!(content.contains("Info message"));
        assert!(content.contains("Error message"));

        // Verify timestamp format (YYYY-MM-DD HH:MM:SS)
        let timestamp_re = regex::Regex::new(r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}").unwrap();
        assert!(timestamp_re.is_match(&content));
    }

    #[test]
    fn test_html_logger_enabled() {
        let logger = HtmlLogger::new(LevelFilter::Info, Vec::new());
        let metadata_info = log::Metadata::builder().level(log::Level::Info).build();
        let metadata_debug = log::Metadata::builder().level(log::Level::Debug).build();
        let metadata_error = log::Metadata::builder().level(log::Level::Error).build();

        assert!(logger.enabled(&metadata_info));
        assert!(!logger.enabled(&metadata_debug)); // Debug is below Info
        assert!(logger.enabled(&metadata_error));
    }

    #[test]
    fn test_html_logger_empty_target() {
        let buffer: Vec<u8> = Vec::new();
        let logger = HtmlLogger::new(LevelFilter::Info, buffer);

        let record = log::Record::builder()
            .args(format_args!("Message with empty target"))
            .target("") // Empty target
            .level(log::Level::Info)
            .build();

        logger.log(&record);
        logger.flush();

        let content = String::from_utf8(logger.get_writable().clone()).expect("Valid UTF-8");
        assert!(!content.contains("Message with empty target")); // Message should not be logged
    }

    #[test]
    fn test_html_logger_thread_id() {
        let buffer: Vec<u8> = Vec::new();
        let logger = HtmlLogger::new(LevelFilter::Info, buffer);

        let record = log::Record::builder()
            .args(format_args!("Message with thread ID"))
            .target("test_target")
            .level(log::Level::Info)
            .build();

        logger.log(&record);
        logger.flush();

        let content = String::from_utf8(logger.get_writable().clone()).expect("Valid UTF-8");

        // Check that the thread ID is present and formatted correctly.  We don't know the exact ID,
        // but we know it should be a number enclosed in square brackets.
        let thread_id_re = regex::Regex::new(r"\[\d+\]").unwrap();
        let thread_id_location = content
            .find("test_target")
            .expect("Target should be present")
            - 17;
        let thread_id_slice = &content[thread_id_location..];
        assert!(
            thread_id_re.is_match(thread_id_slice),
            "Thread ID not formatted correctly. Slice: {}",
            thread_id_slice
        );
    }

    #[test]
    fn test_target_prefix_handling() {
        let buffer: Vec<u8> = Vec::new();
        let logger = HtmlLogger::new(LevelFilter::Info, buffer);

        // Test internal target (starts with "tibberator.")
        let internal_record = log::Record::builder()
            .args(format_args!("Internal message"))
            .target("tibberator.internal.module")
            .level(log::Level::Info)
            .build();
        logger.log(&internal_record);

        // Test external target
        let external_record = log::Record::builder()
            .args(format_args!("External message"))
            .target("external_crate::module")
            .level(log::Level::Info)
            .build();
        logger.log(&external_record);

        logger.flush();

        let content = String::from_utf8(logger.get_writable().clone()).expect("Valid UTF-8");

        // Verify targets are formatted correctly
        assert!(
            content.contains("[tibberator.internal.module]"),
            "Internal target not formatted correctly"
        );
        assert!(
            content.contains("[extern.external_crate::module]"),
            "External target missing 'extern.' prefix"
        );
    }
}
