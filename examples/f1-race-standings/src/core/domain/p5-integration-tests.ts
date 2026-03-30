import { Item } from '../domain/item';
import { ItemRepository } from '../ports/item-repository';

describe('Item Integration Tests', () => {
  let repository: ItemRepository;

  beforeEach(() => {
    repository = new ItemRepository();
  });

  it('should create and retrieve items', async () => {
    const newItem = new Item({ id: 1, name: 'Test Item' });
    await repository.save(newItem);
    const retrievedItem = await repository.findById(1);
    expect(retrievedItem).toEqual(newItem);
  });

  it('should handle duplicate item creation', async () => {
    const item1 = new Item({ id: 1, name: 'Duplicate' });
    await repository.save(item1);
    const item2 = new Item({ id: 1, name: 'Duplicate' });
    await expect(repository.save(item2)).rejects.toThrow('Duplicate item error');
  });

  it('should delete items successfully', async () => {
    const item = new Item({ id: 2, name: 'Delete Me' });
    await repository.save(item);
    await repository.deleteById(2);
    const deleted = await repository.findById(2);
    expect(deleted).toBeUndefined();
  });
});