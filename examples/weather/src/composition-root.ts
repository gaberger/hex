export interface AppContext {
  rootPath: string;
}

export async function createAppContext(rootPath: string): Promise<AppContext> {
  return { rootPath };
}
