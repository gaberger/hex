# Developer-Facing Overview: The Autonomous Loop

## Overview

This document outlines the process by which an operator drives the autonomous loop in our system. The loop consists of several key steps, each performed by different components within the system.

1. **Board Ask**: The operator initiates a request on the board.
2. **Persona Claim**: The system claims the task based on predefined rules.
3. **SOP Ground/Reason/Act/Execute**: The system follows a standard operating procedure (SOP) to perform the task.
4. **Drafter Typed-Tool Emit**: A drafter tool is used to generate content.
5. **Digital-Twin Approval**: The generated content is reviewed and approved by a digital twin.
6. **Executor File Write**: The executor writes the final content to a file.
7. **Commit**: The changes are committed to version control.

## Concrete Example

### Board Message Example

```plaintext
[Board Ask]
Task: Update documentation for the `agentic-dev-roundtrip` process.
Priority: High
Assigned To: CTO
```

### Dashboard Watch Items

1. **Task Status**: Monitor the status of the task on the dashboard to ensure it is being processed correctly.
2. **SOP Execution**: Check the SOP execution logs to verify that each step is completed as expected.
3. **Digital-Twin Review**: Ensure that the digital twin has approved the generated content.
4. **Commit Log**: Verify that the changes have been committed to version control.

## Detailed Steps

### 1. Board Ask

The operator creates a task on the board with specific details such as the task description, priority, and assignee.

### 2. Persona Claim

The system claims the task based on predefined rules and assigns it to the appropriate persona (e.g., CTO).

### 3. SOP Ground/Reason/Act/Execute

The system follows a standard operating procedure (SOP) to perform the task. The SOP includes:

- **Ground**: The context or environment in which the task is executed.
- **Reason**: The rationale for performing the task.
- **Act**: The specific actions to be taken.
- **Execute**: The execution of the actions.

### 4. Drafter Typed-Tool Emit

A drafter tool is used to generate content based on the task details and SOP. The tool emits the generated content in a specified format.

### 5. Digital-Twin Approval

The digital twin reviews the generated content to ensure it meets quality standards and requirements. If approved, the content proceeds to the next step.

### 6. Executor File Write

The executor writes the final content to a file in the appropriate repository (e.g., `docs/specs/agentic-dev-roundtrip.md`).

### 7. Commit

The changes are committed to version control with a descriptive commit message.

## Conclusion

This document provides an overview of the autonomous loop process, from task initiation to completion. By following the outlined steps and monitoring key dashboard items, operators can effectively drive the autonomous loop and ensure that tasks are completed efficiently and accurately.