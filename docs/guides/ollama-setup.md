# Ollama Setup Guide for Hex Autonomous Execution

This guide will walk you through the process of installing and configuring Ollama to work with hex for autonomous execution.

## Prerequisites

- Docker installed on your machine.
- Basic understanding of command line operations.

## Installation Steps

1. **Pull the Ollama Docker Image**

   First, pull the Ollama Docker image from Docker Hub. We will use the `qwen2.5-coder:32b` model for this setup.

   ```bash
   docker pull ollama/qwen2.5-coder:32b
   ```

2. **Run the Ollama Container**

   Run the Ollama container with the necessary configurations. You can map a local directory to the container to persist data or for configuration files.

   ```bash
   docker run -d --name ollama-container -p 11434:11434 -v /path/to/local/data:/data ollama/qwen2.5-coder:32b
   ```

   Replace `/path/to/local/data` with the path to your local directory where you want to store data.

## Configuration Steps

1. **Accessing Ollama**

   You can access the Ollama API by navigating to `http://localhost:11434` in your web browser or using tools like `curl`.

2. **Setting Up Hex Autonomous Execution**

   To configure Ollama for hex autonomous execution, you need to set up a configuration file. Create a file named `config.yaml` in the mapped directory with the following content:

   ```yaml
   model: qwen2.5-coder:32b
   api_key: your_api_key_here
   tasks:
     - name: hex_execution
       command: "hex run autonomous"
       schedule: "0 * * * *"  # Runs every hour
   ```

   Replace `your_api_key_here` with your actual API key.

3. **Applying Configuration**

   Ensure that the configuration file is correctly placed in the mapped directory and accessible by the Ollama container. The container will automatically pick up the configuration and start executing tasks as per the schedule.

## Conclusion

You have now successfully installed and configured Ollama for hex autonomous execution. You can monitor the execution logs and manage tasks through the Ollama API.

For more advanced configurations and options, refer to the [Ollama Documentation](https://ollama.com/docs).