import { DescriptionService } from '../ports/description-service';
import { DescriptionRepository } from '../ports/description-repository';
import { MySqlDescriptionRepository } from '../adapters/secondary/mysql-description-repository';
import { DescriptionController } from '../adapters/primary/description-controller';

// Composition root
const descriptionRepository: DescriptionRepository = new MySqlDescriptionRepository();
const descriptionService: DescriptionService = new DescriptionService(descriptionRepository);
const descriptionController = new DescriptionController(descriptionService);

// Export the wired components
export { descriptionController };