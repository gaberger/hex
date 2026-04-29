# Ollama Setup Guide for Hex Autonomous Execution

## Introduction

This guide will walk you through the process of installing and configuring Ollama to enable autonomous execution with Hex. We'll be using the `qwen2.5-coder:32b` model as an example.

## Prerequisites

- Docker installed on your machine.
- Basic knowledge of command-line operations.

## Installation Steps

1. **Pull the Ollama Docker Image**

   First, you need to pull the Ollama Docker image from Docker Hub. Open your terminal and run:

   ```bash
   docker pull ollama/ollama:latest
   ```

2. **Run the Ollama Container**

   Once the image is downloaded, you can start a container using the following command:

   ```bash
   docker run -d --name ollama-container -p 11434:11434 ollama/ollama:latest
   ```

   This command runs Ollama in detached mode and maps port 11434 on your host to port 11434 in the container.

## Configuration Steps

1. **Access the Ollama Container**

   To configure Ollama, you need to access the running container:

   ```bash
   docker exec -it ollama-container /bin/sh
   ```

2. **Download and Install the Model**

   Inside the container, download and install the `qwen2.5-coder:32b` model by running:

   ```bash
   ollama pull qwen2.5-coder:32b
   ```

3. **Configure Hex Autonomous Execution**

   To enable autonomous execution with Hex, you need to create a configuration file. Create a new file named `config.yaml` and add the following content:

   ```yaml
   model:
     name: qwen2.5-coder:32b
     parameters:
       max_tokens: 4096
       temperature: 0.7

   hex:
     enabled: true
     execution_mode: autonomous
   ```

   Save the file and exit the editor.

4. **Start Hex Execution**

   Finally, start Hex execution by running:

   ```bash
   ollama run --config config.yaml
   ```

## Conclusion

You have now successfully installed and configured Ollama for hex autonomous execution using the `qwen2.5-coder:32b` model. You can further customize the configuration file to suit your specific needs.

For more information, refer to the [Ollama documentation](https://ollama.com/docs).