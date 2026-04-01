Launch a new agent to handle complex, multi-step tasks autonomously. Each agent type has specific capabilities and tools available to it.

When the agent is done, it returns a single message — the result is not visible to the user; send a text summary.

You can run agents in the background using the run_in_background parameter. When running in the background, you will be notified when it completes — do NOT poll or sleep.

Background agents that edit files MUST use mode=bypassPermissions.
