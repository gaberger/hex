 # Architecture Health Document

This document provides an overview of our architecture's health status. It includes metrics that indicate whether we are in a healthy or unhealthy state regarding architectural design and implementation.

## Key Metrics

1. **Code Quality:** The percentage of clean code, as measured by the number of files with no syntax errors (excluding comments) divided by the total number of source code files. [Source: GitHub Actions](https://github.com/settings/security)
2. **Collaboration Health:** The percentage of pull requests and issues that have passed quality gates and are open, as measured using GitHub's built-in stats feature ([Source: GitHub Stats](https://status.github.com/)).
3. **Technical Debt:** The number of days it would take to resolve technical debt based on the number of affected lines of code multiplied by an appropriate factor (e.g., 1 day per 100 LOC, or some other value chosen for this purpose). [Source: GitHub Actions](https://github.com/settings/security)
4. **Code Coverage:** The percentage of covered statements in our tests divided by the total number of testable statements multiplied by a factor that accounts for the complexity of the code (e.g., 1% per LOC, or some other value chosen for this purpose). [Source: GitHub Actions](https://github.com/settings/security)
5. **Test Coverage:** The percentage of covered tests in our test suite divided by the total number of available test cases multiplied by a factor that accounts for the complexity of the code (e.g., 1% per LOC, or some other value chosen for this purpose). [Source: GitHub Actions](https://github.com/settings/security)
6. **Average Developer Productivity:** The average time it takes to complete new features divided by the number of days in a given period multiplied by an appropriate factor that accounts for team size (e.g., hours per developer * 20% or some other value chosen for this purpose). [Source: GitHub Stats](https://status.github.com/)
7. **Quality Gate Health:** The percentage of pull requests and issues that have passed quality gates divided by the total number, as measured using GitHub's built-in stats feature ([Source: GitHub Stats](https://status.github.com/)).
8. **Critical Dependency Health:** A metric indicating whether our dependencies are up-to-date or not; this could be based on a percentage of outdated packages relative to the total number, or some other measure chosen for this purpose. [Source: GitHub Actions](https://github.com/settings/security)
9. **Lint and Static Analysis Health:** A metric indicating whether our code adheres to coding standards using tools like ESLint and static analysis tools (e.g., SonarQube). [Source: Tools configured in CI pipelines]
10. **Other Metrics** : Any other relevant metrics that could help assess the architecture's health status, such as the number of security vulnerabilities or compliance with architectural principles. These should be added if deemed necessary and based on established processes for identifying and reporting potential issues. [Source: Tools configured in CI pipelines]

## Analysis

This document provides a high-level analysis of our architecture's health by combining these metrics into an overall score, which is used to inform decisions about whether we need to take proactive steps to improve the architecture or if it is healthy as is. The analysis should be done using a scoring model that takes into account each metric's weight (as defined above) and provides an aggregated score indicating our architectural health status.

## Action Plan

The action plan includes recommendations for improving any metrics below certain thresholds, such as increasing code coverage or fixing technical debt. It may also include suggestions for implementing additional metrics if deemed necessary based on established processes for identifying and reporting potential issues. The action plan should be agreed upon by the relevant stakeholders and communicated to the entire team in a clear manner.

The `docs/specs/architecture\_health.md` file is generated from GitHub Actions, so you can find more details about our architecture health analysis pipeline [here](https://github.com/integration-with-github/actions).