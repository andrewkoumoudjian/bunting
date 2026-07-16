export default {
  fetch() {
    return Response.json(
      { error: "raw workerd smoke does not emulate D1" },
      { status: 501 },
    );
  },
};
