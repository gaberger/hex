import { ItemDescription } from "../domain/item-description.js";
import { ItemRepository } from "../ports/item-repository.js";
import { ItemService } from "../ports/item-service.js";
import { MemoryItemRepository } from "../adapters/secondary/memory-item-repository.js";

const itemRepository: ItemRepository = new MemoryItemRepository();
const itemService: ItemService = new ItemService(itemRepository);

export { itemService, itemRepository };