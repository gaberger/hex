import { AuditLogger } from '../../ports/secondary/AuditLogger.js';
import winston from 'winston';

export class WinstonAuditLoggerAdapter implements AuditLogger {
  private readonly logger: winston.Logger;

  constructor() {
    this.logger = winston.createLogger({
      level: 'info',
      format: winston.format.json(),
      transports: [
        new winston.transports.File({ filename: 'audit.log' }),
      ],
    });
  }

  log(event: string, data: object): void {
    this.logger.info(event, data);
  }
}

export default WinstonAuditLoggerAdapter;