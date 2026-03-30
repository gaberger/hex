import { DomainType } from '../domain/domain-type';

export interface CLICommand {
  execute(): DomainType;
}