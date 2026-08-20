 # Pipeline Roundtrip v2 Documentation
This document outlines the details required for implementing a round trip pipeline in our system. A round trip pipeline is designed to allow us to take an artifact from one stage of the system and send it through the entire pipeline back to its original stage, then repeat this process iteratively until the artifact reaches its final destination (i.e., the production environment).

This document describes how to define a round trip pipeline in terms of stages (`stages`), artifacts (`artifacts`), transformations (`transformations`), and actions (`actions`). 

## Stages
Our system supports various stages that can be used for different purposes, such as staging, testing, or development. Each stage should have its own set of rules on what artifacts are allowed to enter and leave the stage, how artifacts may transform (if any), and which transformations are available within this stage. Please refer to each stage's documentation (`docs/stages/<stage>-<version>.md`) for more details.

## Artifacts
Each stage should have its own set of required artifacts that can be used across all stages in the pipeline round trip. For example, a common artifact might be a versioned configuration file or an environment variable dictionary. The list of required artifacts is defined as: `<artifact-name>-<version>.` 

## Transformations and Actions
A transformation takes one or more input artifacts (`<artifact-name>-<version>`) and outputs zero or more output artifacts, each with its own version number. For example, the `unpack` transformation transforms a compressed artifact into its original format. An action is an executable program that performs some specific task on the artifacts within a stage. 

Here are examples of transformations:
- `<artifact>`: A common artifact used across stages.
  + `unpack`: Compresses and decompresses the artifact if it's compressed, otherwise just copies it.
  + `convert_to_json`: Converts an artifact into a JSON format if needed.

Here are examples of actions:
- `<stage>`: A stage where transformations can take place.
  + `<action>` : An executable program that performs some specific task on the artifacts within this stage. For example, a test phase may have an action to run automated tests or deploy the artifact into production for deployment testing. 

Here's how you define a round trip pipeline:
```yaml
round_trip_pipeline:
  - stages: [staging, testing, production]
    # ... (other config options like artifacts and actions)
```
This document outlines all the necessary components of our round trip pipeline. Please refer to it for more information on implementing a round trip pipeline in our system.