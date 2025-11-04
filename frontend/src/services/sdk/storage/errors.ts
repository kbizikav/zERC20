export class StealthError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'StealthError';
  }
}

export class AnnouncementIgnoredError extends StealthError {
  constructor(reason: string) {
    super(`announcement ignored: ${reason}`);
    this.name = 'AnnouncementIgnoredError';
  }
}
