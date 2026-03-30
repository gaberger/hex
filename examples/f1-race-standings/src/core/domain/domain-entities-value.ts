import { EmailAddress} from './email-address';
import { UserId } from './user-id';
import { Name } from './name';

export class User {
  constructor(
    public readonly id: UserId,
    public readonly email: EmailAddress,
    public readonly name: Name,
  ) {
    if (!email.isValid()) throw new Error('Invalid email');
    if (!name.isValid()) throw new Error('Invalid name');
  }

  updateEmail(newEmail: EmailAddress): void {
    if (!newEmail.isValid()) throw new Error('Invalid email');
    this.email = newEmail;
  }
}

export class EmailAddress {
  constructor(public readonly value: string) {}

  isValid(): boolean {
    const regex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
    return regex.test(this.value);
  }
}

export class UserId {
  constructor(public readonly value: string) {}

  isValid(): boolean {
    return /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(this.value);
  }
}

export class Name {
  constructor(public readonly value: string) {}

  isValid(): boolean {
    return this.value.trim().length >= 2;
  }
}