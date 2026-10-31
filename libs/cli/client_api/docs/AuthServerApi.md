# \AuthServerApi

All URIs are relative to *http://localhost*

Method | HTTP request | Description
------------- | ------------- | -------------
[**server_auth_callback**](AuthServerApi.md#server_auth_callback) | **GET** /api/v1/auth/callback | Callback from the proxy's identity provider, for the SSO session (RFC 0010)
[**server_auth_login**](AuthServerApi.md#server_auth_login) | **GET** /api/v1/auth/login | Start the SSO session to the proxy (RFC 0010)
[**server_auth_refresh**](AuthServerApi.md#server_auth_refresh) | **POST** /api/v1/auth/refresh | Renew the SSO session to the proxy (RFC 0010)
[**server_auth_token**](AuthServerApi.md#server_auth_token) | **POST** /api/v1/auth/token | Redeem the single-use code of the SSO session (RFC 0010)



## server_auth_callback

> server_auth_callback(state, code, error)
Callback from the proxy's identity provider, for the SSO session (RFC 0010)

Not meant to be opened directly. On success, redirects to the CLI's loopback `redirect_uri` with a single-use code, never a token.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**state** | **String** | The correlation id this proxy generated at `/auth/login`. | [required] |
**code** | Option<**String**> | Authorization code from the proxy's identity provider; absent when it answers with an `error` instead. |  |
**error** | Option<**String**> | The identity provider's error (`access_denied`…), sent back to the CLI as `access_denied`. |  |

### Return type

 (empty response body)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: Not defined

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## server_auth_login

> server_auth_login(redirect_uri, code_challenge, state)
Start the SSO session to the proxy (RFC 0010)

Redirects the browser to the proxy's identity provider; the CLI receives a single-use code on its loopback `redirect_uri`, redeemed at `/auth/token`.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**redirect_uri** | **String** | The CLI's loopback listener (`http://127.0.0.1:<port>/…`, `localhost` or `[::1]`); anything else is refused. | [required] |
**code_challenge** | **String** | RFC 7636 `S256` challenge of the CLI's verifier. | [required] |
**state** | Option<**String**> | Opaque value echoed back verbatim on redirect. |  |

### Return type

 (empty response body)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: Not defined

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## server_auth_refresh

> models::SessionTokens server_auth_refresh(refresh_token)
Renew the SSO session to the proxy (RFC 0010)

The identity provider stays the only judge of the refresh token. A refused one counts toward the unscoped ban; an unreachable or misconfigured provider does not.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**refresh_token** | **String** | The refresh token of the session; in the body, never in the URL. | [required] |

### Return type

[**models::SessionTokens**](SessionTokens.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: application/x-www-form-urlencoded
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## server_auth_token

> models::SessionTokens server_auth_token(code, code_verifier, redirect_uri)
Redeem the single-use code of the SSO session (RFC 0010)

The CLI trades the code `/auth/callback` sent to its loopback, with the PKCE verifier, for the session. A code works once, whatever the outcome.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**code** | **String** | The single-use code `/auth/callback` sent to the loopback listener. | [required] |
**code_verifier** | **String** | The PKCE verifier of the `code_challenge` given to `/auth/login`. | [required] |
**redirect_uri** | **String** | The `redirect_uri` given to `/auth/login`. | [required] |

### Return type

[**models::SessionTokens**](SessionTokens.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: application/x-www-form-urlencoded
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

