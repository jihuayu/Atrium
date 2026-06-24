export interface ApiFieldError {
  resource: string;
  field: string;
  code: string;
}

export class ApiError extends Error {
  readonly status: number;
  readonly errors: ApiFieldError[];

  constructor(status: number, message: string, errors: ApiFieldError[] = []) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.errors = errors;
  }

  static badRequest(message: string) {
    return new ApiError(400, message);
  }

  static unauthorized() {
    return new ApiError(401, "Requires authentication");
  }

  static forbidden(message: string) {
    return new ApiError(403, message);
  }

  static notFound(resource: string) {
    return new ApiError(404, `${resource} not found`);
  }

  static validation(resource: string, field: string, code: string) {
    return new ApiError(422, "Validation Failed", [{ resource, field, code }]);
  }

  static internal(message: string) {
    return new ApiError(500, message);
  }

  nativeBody() {
    const error =
      this.status === 400
        ? "bad_request"
        : this.status === 401
          ? "unauthorized"
          : this.status === 403
            ? "forbidden"
            : this.status === 404
              ? "not_found"
              : this.status === 409
                ? "conflict"
                : this.status === 422
                  ? "validation_failed"
                  : "internal_error";
    return { error, message: this.message };
  }
}

export function asApiError(error: unknown): ApiError {
  if (error instanceof ApiError) return error;
  if (error instanceof Error) return ApiError.internal(error.message);
  return ApiError.internal("internal error");
}
