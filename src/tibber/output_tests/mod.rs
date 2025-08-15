#[cfg(test)]
mod tests {
    use crate::tibber::output::{OutputConfig, OutputType, display_bar_graph};

    #[test]
    fn test_is_silent() {
        let mut config = OutputConfig::default();
        assert!(!config.is_silent());
        config.output_type = OutputType::Silent;
        assert!(config.is_silent());
    }

    #[test]
    fn test_bar_graph_with_positive_values() {
        // Create test data with only positive values
        let mut data = vec![0.0; 24];
        for i in 0..24 {
            data[i] = (i as f64).powf(1.5); // Non-linear growth for variety
        }

        let mut output = Vec::new();
        assert!(display_bar_graph(&data, "Positive Values Test", &mut output).is_ok());

        let result = String::from_utf8(output).unwrap();

        // Basic assertions
        assert!(result.contains("Positive Values Test"));

        // Check if the maximum value is displayed
        let max_value = data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let formatted_max = format!("{:.3}", max_value);
        assert!(result.contains(&formatted_max));
    }

    #[test]
    fn test_bar_graph_with_negative_values() {
        // Create test data with only negative values
        let mut data = vec![0.0; 24];
        for i in 0..24 {
            data[i] = -1.0 * (i as f64).powf(1.5); // Non-linear decline for variety
        }

        let mut output = Vec::new();
        assert!(display_bar_graph(&data, "Negative Values Test", &mut output).is_ok());

        let result = String::from_utf8(output).unwrap();

        // Basic assertions
        assert!(result.contains("Negative Values Test"));

        // Check if the minimum value is displayed
        let min_value = data.iter().cloned().fold(f64::INFINITY, f64::min);
        let formatted_min = format!("{:.3}", min_value);
        assert!(result.contains(&formatted_min));
    }

    #[test]
    fn test_bar_graph_with_mixed_values() {
        // Create test data with both positive and negative values
        let mut data = vec![0.0; 24];
        for i in 0..24 {
            data[i] = 10.0 * ((i as f64) - 12.0); // Values around zero (-120 to +110)
        }

        let mut output = Vec::new();
        assert!(display_bar_graph(&data, "Mixed Values Test", &mut output).is_ok());

        let result = String::from_utf8(output).unwrap();

        // Basic assertions
        assert!(result.contains("Mixed Values Test"));

        // Since we have both positive and negative values, check for zero line indicators
        assert!(result.contains("─  ")); // Horizontal line for zero
    }

    #[test]
    fn test_bar_graph_with_invalid_data_length() {
        // Test with incorrect data length
        let data = vec![1.0, 2.0, 3.0]; // Only 3 values instead of 24

        let mut output = Vec::new();
        assert!(display_bar_graph(&data, "Invalid Data Test", &mut output).is_err());
    }

    #[test]
    fn test_bar_graph_with_constant_values() {
        // All values are the same
        let data = vec![5.0; 24];

        let mut output = Vec::new();
        assert!(display_bar_graph(&data, "Constant Values Test", &mut output).is_ok());

        let result = String::from_utf8(output).unwrap();

        // Basic assertions
        assert!(result.contains("Constant Values Test"));

        // Check if bars are drawn despite constant values
        assert!(result.contains("█  ")); // Should still draw bar segments
    }
}