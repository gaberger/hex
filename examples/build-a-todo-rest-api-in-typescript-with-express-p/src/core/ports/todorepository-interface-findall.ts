export interface TodoRepository {
  create(item: any): Promise<any>;
  findAll(): Promise<any[]>;
  delete(id: number): Promise<boolean>;
}