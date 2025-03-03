# Tibberator

Tibberator is a Rust application that connects to the Tibber API and provides basic power consumption data. It's designed to help users monitor their energy usage efficiently.

## Features

- Retrieve personalized data using the user's access token.
- Display power consumption information for a specific home using the home ID.
- Automatically load demo token and home ID if no parameters are initially set.
- Store user-provided parameters in a configuration file for future use.

## Usage

1. **Installation**:
   - Make sure you have Rust installed on your system.
   - Clone this repository: `git clone https://github.com/babelfischchen/tibberator.git`
   - Navigate to the project directory: `cd tibberator`

2. **Building and Running**:
   - Build the project: `cargo build`
   - Run Tibberator with default demo parameters: `cargo run`
   - Specify your access token and home ID using command-line flags:
     ```
     cargo run -- --token <access_token> --home_id <home_id>
     ```

3. **Configuration File**:
   - Once you provide the parameters, Tibberator will store them in a configuration file (`config.yaml`).
   - In subsequent runs, Tibberator will automatically load the stored parameters from the config file.

## Command-Line Flags

- `--token <access_token>`: Provides the user's access token for personalized data retrieval.
- `--home_id <home_id>`: Specifies the user's home ID.
- `-d` or `--dislay <display_mode`: Defines how Tibberator should display the data (possible values "consumption" or "prices").


Remember to replace `<access_token>` and `<home_id>` with actual values when using Tibberator.

## Contributing

Contributions are welcome! Feel free to open issues or submit pull requests on the [GitHub repository](https://github.com/babelfischchen/tibberator).

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.
