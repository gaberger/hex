# Ollama Setup Guide for Hex Autonomous Execution

## Installation

1. **Download and Install Docker**:
   Ensure Docker is installed on your system. You can download it from [Docker's official website](https://www.docker.com/products/docker-desktop).

2. **Pull the Ollama Image**:
   Open a terminal and pull the Ollama image using Docker.
   ```bash
   docker pull ollama/ollama:latest
   ```

3. **Run the Ollama Container**:
   Start the Ollama container with the necessary configurations.
   ```bash
   docker run -d --name ollama-container -p 11434:11434 ollama/ollama:latest
   ```

## Configuration

1. **Access the Ollama Container**:
   Enter the running container to configure it.
   ```bash
   docker exec -it ollama-container /bin/bash
   ```

2. **Set Up Hex Autonomous Execution**:
   Configure Ollama for hex autonomous execution by setting up the necessary environment variables and configurations.

3. **Download the Model**:
   Download the `qwen2.5-coder:32b` model.
   ```bash
   ollama pull qwen2.5-coder:32b
   ```

4. **Configure Execution Parameters**:
   Set up any additional parameters required for hex autonomous execution in the Ollama configuration file.

5. **Start Hex Autonomous Execution**:
   Begin the autonomous execution process using the configured model.
   ```bash
   ollama run qwen2.5-coder:32b --autonomous-execution
   ```

## Verification

1. **Check Logs**:
   Verify that Ollama is running correctly by checking the logs.
   ```bash
   docker logs ollama-container
   ```

2. **Test Execution**:
   Test the hex autonomous execution to ensure it is functioning as expected.

This guide provides a step-by-step process for installing and configuring Ollama for hex autonomous execution using Docker.