import { AuthAdapter } from '../../ports/AuthAdapter.js';
import { UserRole } from '../../domain/UserRole.js';
import * as jwt from 'jsonwebtoken';

interface JwtPayload {
  userId: string;
  role: UserRole;
}

class JwtAuthAdapter implements AuthAdapter {
  private readonly secretKey: string;
  private readonly tokenExpiration: string;

  constructor(secretKey: string, tokenExpiration: string) {
    this.secretKey = secretKey;
    this.tokenExpiration = tokenExpiration;
  }

  async generateToken(userId: string, role: UserRole): Promise<string> {
    const payload: JwtPayload = { userId, role };
    return jwt.sign(payload, this.secretKey, { expiresIn: this.tokenExpiration });
  }

  async verifyToken(token: string): Promise<{ userId: string; role: UserRole } | null> {
    try {
      const decoded = jwt.verify(token, this.secretKey) as JwtPayload;
      return { userId: decoded.userId, role: decoded.role };
    } catch (error) {
      return null;
    }
  }

  async validateRole(token: string, requiredRole: UserRole): Promise<boolean> {
    const decoded = await this.verifyToken(token);
    if (!decoded) return false;
    return decoded.role === requiredRole;
  }
}

export { JwtAuthAdapter };