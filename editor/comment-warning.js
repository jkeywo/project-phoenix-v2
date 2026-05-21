export class CommentWarning {
  constructor() {
    this.acknowledged = false;
  }

  shouldWarn(rawText) {
    if (this.acknowledged) return false;
    return rawText.includes('#');
  }

  acknowledge() {
    this.acknowledged = true;
  }

  isAcknowledged() {
    return this.acknowledged;
  }

  reset() {
    this.acknowledged = false;
  }
}
