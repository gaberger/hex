// wp-inference-quality-gate — Add inference quality gate: test provider connectivity and prune failing providers
export interface InferenceQualityGate {
  testProviderConnectivity(): Promise<boolean>;
  pruneFailingProviders(): Promise<void>;
}